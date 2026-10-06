use releash_lib::test_support::client_api_acceptance::*;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

struct Fixture {
    host: ClientApiAcceptanceHost,
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
        let host = ClientApiAcceptanceHost::start(data.path(), branch);
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
    fn args(&self) -> Value {
        json!({"repoPath":self.repo.path().to_str().unwrap()})
    }
}

async fn request(client: &NativeClient, frame: Value) -> Value {
    let result = read_current_branch(client, frame.get("args").cloned().unwrap_or(json!({})))
        .await
        .unwrap();
    json!({"request_id":frame["request_id"],"result":result})
}

#[tokio::test(flavor = "multi_thread")]
async fn test_クライアントconnect_認証と相関を保つ() {
    // Given
    let fixture = Fixture::new().await;
    for (token, origin, status) in [
        ("wrong", "tauri://localhost", 401),
        (fixture.token.as_ref(), "https://evil.example", 403),
    ] {
        let response = reqwest::Client::new()
            .post(format!(
                "{}/releash.client.v1.ClientService/UpdateExternalEditor",
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
    let client = fixture.client();
    // When
    let response = request(
        &client,
        json!({"request_id":"one","command":"current-branch","args":fixture.args()}),
    )
    .await;
    assert_eq!(response["request_id"], "one");
    assert_eq!(response["result"], "ws-branch");
}

#[tokio::test]
async fn test_クライアント認証_wsで有効な非master_tokenはhttp入口で拒否する() {
    // Given
    let fixture = Fixture::new().await;
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
}

#[tokio::test]
async fn test_クライアント購読_失敗と不正引数の分類と説明を保持する() {
    let fixture = Fixture::new().await;
    let client = fixture.client();
    for (args, expected) in [
        (json!({}), connectrpc::ErrorCode::InvalidArgument),
        (
            json!({"repoPath":"/missing-releash-repository"}),
            connectrpc::ErrorCode::Internal,
        ),
    ] {
        let error = read_current_branch(&client, args).await.unwrap_err();
        assert_eq!(error.code, expected);
        assert!(error
            .message
            .as_ref()
            .is_some_and(|message| !message.is_empty()));
    }
}

#[tokio::test]
async fn test_connect_不正protoを拒否する() {
    let fixture = Fixture::new().await;
    let http = reqwest::Client::new();
    let response = http
        .post(format!(
            "{}/releash.client.v1.ClientService/UpdateExternalEditor",
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
}

#[tokio::test]
async fn test_生成client_connectとgrpcとgrpcwebが同じserviceを呼べる() {
    use connectrpc::client::{ClientConfig, HttpClient};
    use connectrpc::Protocol;
    let fixture = Fixture::new().await;
    for protocol in [Protocol::Connect, Protocol::Grpc, Protocol::GrpcWeb] {
        let config = ClientConfig::new(fixture.url.parse().unwrap())
            .with_protocol(protocol)
            .with_default_header("authorization", format!("Bearer {}", fixture.token));
        let transport = if matches!(protocol, Protocol::Grpc) {
            HttpClient::plaintext_http2_only()
        } else {
            HttpClient::plaintext()
        };
        let client = rpc::ClientServiceClient::new(transport, config);
        assert_eq!(
            read_current_branch(&client, fixture.args()).await.unwrap(),
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

    fn create(&self, repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
        BranchGateway.create(repo_path, branch_name)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_connect_defaultの枠と待ち行列を超える要求を拒否する() {
    let started = Arc::new(tokio::sync::Notify::new());
    let (resume, receiver) = std::sync::mpsc::channel();
    let fixture = Fixture::with_branch(Arc::new(PausedBranch {
        started: started.clone(),
        resume: std::sync::Mutex::new(receiver),
    }))
    .await;
    let mut pending = tokio::task::JoinSet::new();
    for _ in 0..92 {
        let client = fixture.client();
        let args = json!({"filePath": fixture.repo.path().to_str().unwrap()});
        pending.spawn(async move { request_client(&client, "get_language_from_path", args).await });
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
    for _ in 0..91 {
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
