use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;

use releashd::test_support::integration::client_protocol::{rpc, wire};
fn to_wire<T: prost::Message + Default>(
    value: &impl buffa::Message,
) -> Result<T, connectrpc::ConnectError> {
    T::decode(buffa::Message::encode_to_vec(value).as_slice())
        .map_err(|e| connectrpc::ConnectError::internal(e.to_string()))
}
struct Socket {
    client: rpc::ClientServiceClient<connectrpc::client::HttpClient>,
}
type StateStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = wire::state_payload::Value> + Send>>;
async fn subscribe_state(socket: &Socket, target: &str, args: Vec<String>) -> StateStream {
    let client_id = uuid::Uuid::new_v4().to_string();
    let mut stream = socket
        .client
        .open_state_stream(rpc::OpenStateStreamRequest {
            client_id: client_id.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    stream
        .message::<rpc::StateSubscriptionEvent>()
        .await
        .unwrap();
    socket
        .client
        .start_state_subscription(rpc::StartStateSubscriptionRequest {
            client_id,
            subscription_id: uuid::Uuid::new_v4().to_string(),
            target: target.into(),
            args,
            ..Default::default()
        })
        .await
        .unwrap();
    Box::pin(futures_util::stream::unfold(
        stream,
        |mut stream| async move {
            loop {
                let item = stream
                    .message::<rpc::StateSubscriptionEvent>()
                    .await
                    .unwrap()?
                    .to_owned_message();
                let event: wire::StateSubscriptionEvent = to_wire(&item).unwrap();
                let payload = match event.event {
                    Some(wire::state_subscription_event::Event::Snapshot(value)) => Some(value),
                    Some(wire::state_subscription_event::Event::Change(value)) => value.payload,
                    Some(wire::state_subscription_event::Event::Failure(error)) => {
                        panic!(
                            "state subscription failed: {} ({})",
                            error.message, error.code
                        )
                    }
                    _ => None,
                };
                if let Some(wire::StatePayload { value: Some(value) }) = payload {
                    return Some((value, stream));
                }
            }
        },
    ))
}
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn start(directory: &Path) -> (Daemon, Socket) {
    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_releashd"))
            .arg("--data-dir")
            .arg(directory)
            .env_remove("RELEASH_DATA_DIR")
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let discovery = loop {
        assert!(daemon.0.try_wait().unwrap().is_none(), "daemon exited");
        if let Ok(bytes) = std::fs::read(directory.join("client-api.json")) {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                if value["pid"].as_u64() == Some(u64::from(daemon.0.id())) {
                    break value;
                }
            }
        }
        assert!(Instant::now() < deadline, "daemon discovery timed out");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let config = connectrpc::client::ClientConfig::new(
        format!("http://127.0.0.1:{}", discovery["port"])
            .parse()
            .unwrap(),
    )
    .with_default_header(
        "authorization",
        format!("Bearer {}", discovery["token"].as_str().unwrap()),
    );
    (
        daemon,
        Socket {
            client: rpc::ClientServiceClient::new(
                connectrpc::client::HttpClient::plaintext(),
                config,
            ),
        },
    )
}

// B-005, B-006, B-007
#[tokio::test(flavor = "multi_thread")]
async fn test_リポジトリ名_originと一覧の同名から決めて配信する() {
    // Given
    let data = tempfile::tempdir().unwrap();
    let repos = tempfile::tempdir().unwrap();
    let (_daemon, socket) = start(data.path()).await;
    let mut expected = Vec::new();
    for (folder, origin, name) in [
        (
            "work",
            "https://github.com/siro33950/releash.git",
            "releash",
        ),
        (
            "articles",
            "https://gitlab.com/example/different.git",
            "articles",
        ),
        (
            "other",
            "git@github.com:example/releash.git",
            "example/releash",
        ),
    ] {
        let path = repos.path().join(folder);
        let git = git2::Repository::init(&path).unwrap();
        let signature = git2::Signature::now("acceptance", "acceptance@example.com").unwrap();
        let tree_id = git.index().unwrap().write_tree().unwrap();
        let tree = git.find_tree(tree_id).unwrap();
        git.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
        git.remote("origin", origin).unwrap();
        // When
        socket
            .client
            .add_repo_path(rpc::AddRepoPathRequest {
                path: Some(path.to_str().unwrap().into()),
                ..Default::default()
            })
            .await
            .unwrap();
        expected.push((path.to_string_lossy().into_owned(), name.to_string()));
        if folder == "other" {
            expected[0].1 = "siro33950/releash".into();
        }
        let mut stream = subscribe_state(&socket, "workspaces", vec![]).await;
        // Then
        let mut observed = Vec::new();
        let result = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(value) = stream.next().await {
                if let wire::state_payload::Value::Workspaces(snapshot) = value {
                    observed = snapshot
                        .repositories
                        .unwrap()
                        .items
                        .into_iter()
                        .map(|row| (row.path.unwrap(), row.name.unwrap()))
                        .collect();
                    if expected.iter().all(|row| observed.contains(row)) {
                        return;
                    }
                }
            }
            panic!("workspace stream ended");
        })
        .await;
        assert!(
            result.is_ok(),
            "expected {expected:?}, observed {observed:?}"
        );
    }
}

// B-016
#[tokio::test(flavor = "multi_thread")]
async fn test_ターミナル起動コマンド_確定した値を再購読して取得する() {
    // Given
    let data = tempfile::tempdir().unwrap();
    let (_daemon, socket) = start(data.path()).await;
    // When
    socket
        .client
        .update_terminal_startup_command(rpc::UpdateTerminalStartupCommandRequest {
            command: Some("echo accepted".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    let mut stream = subscribe_state(&socket, "terminal-startup-command", vec![]).await;
    let value = tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .unwrap()
        .unwrap();
    let wire::state_payload::Value::TerminalStartupCommand(command) = value else {
        panic!("terminal command payload");
    };
    assert_eq!(command.value.as_deref(), Some("echo accepted"));
}
