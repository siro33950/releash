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
    git: rpc::GitServiceClient<connectrpc::client::HttpClient>,
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
            git: rpc::GitServiceClient::new(
                connectrpc::client::HttpClient::plaintext(),
                config.clone(),
            ),
            client: rpc::ClientServiceClient::new(
                connectrpc::client::HttpClient::plaintext(),
                config,
            ),
        },
    )
}

struct Fixture {
    _daemon: Daemon,
    socket: Socket,
    root: tempfile::TempDir,
    repo: std::path::PathBuf,
    origin: std::path::PathBuf,
}

fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("work");
        let origin = root.path().join("origin.git");
        let data = root.path().join("data");
        for path in [&repo, &origin, &data] {
            std::fs::create_dir(path).unwrap();
        }
        git(&origin, &["init", "--bare", "-b", "main"]);
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["config", "user.name", "Acceptance"]);
        git(
            &repo,
            &["config", "user.email", "acceptance@example.invalid"],
        );
        git(&repo, &["config", "commit.gpgsign", "false"]);
        git(&repo, &["config", "pull.rebase", "false"]);
        git(&repo, &["commit", "--allow-empty", "-m", "initial"]);
        git(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(&repo, &["push", "-u", "origin", "main"]);
        let (daemon, socket) = start(&data).await;
        socket
            .client
            .add_repo_path(rpc::AddRepoPathRequest {
                path: Some(repo.to_string_lossy().into_owned()),
                ..Default::default()
            })
            .await
            .unwrap();
        Self {
            _daemon: daemon,
            socket,
            root,
            repo,
            origin,
        }
    }

    fn path(&self) -> String {
        self.repo.to_string_lossy().into_owned()
    }

    fn write(&self, name: &str, content: &str) {
        let path = self.repo.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn commit(&self, name: &str, content: &str, message: &str) -> String {
        self.write(name, content);
        git(&self.repo, &["add", name]);
        git(&self.repo, &["commit", "-m", message]);
        git(&self.repo, &["rev-parse", "HEAD"])
    }

    async fn snapshot(&self, search: Option<&str>) -> wire::SourceControlSnapshot {
        let mut args = vec![self.path()];
        if let Some(search) = search {
            args.push(format!("search={search}"));
        }
        let mut stream = subscribe_state(&self.socket, "source-control", args).await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(value) = stream.next().await {
                if let wire::state_payload::Value::SourceControl(snapshot) = value {
                    return snapshot;
                }
            }
            panic!("source-control stream closed");
        })
        .await
        .expect("source-control snapshot timed out")
    }

    async fn stage(&self, paths: &[&str]) {
        self.socket
            .git
            .git_stage(rpc::GitStageRequest {
                repo_path: Some(self.path()),
                paths: Some(rpc::Liststring {
                    items: paths.iter().map(|s| (*s).into()).collect(),
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }

    async fn unstage(&self, paths: &[&str]) {
        self.socket
            .git
            .git_unstage(rpc::GitUnstageRequest {
                repo_path: Some(self.path()),
                paths: Some(rpc::Liststring {
                    items: paths.iter().map(|s| (*s).into()).collect(),
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }

    async fn push(&self, force: bool) {
        self.socket
            .git
            .git_push(rpc::GitPushRequest {
                worktree_path: self.path(),
                force_with_lease: force,
                ..Default::default()
            })
            .await
            .unwrap();
    }
}

fn tracking(snapshot: &wire::SourceControlSnapshot) -> &wire::SourceControlTracking {
    let Some(wire::source_control_snapshot::BranchState::Tracking(value)) = &snapshot.branch_state
    else {
        panic!("expected upstream: {snapshot:?}");
    };
    value
}

fn operation(snapshot: &wire::SourceControlSnapshot) -> &wire::SourceControlOperation {
    let Some(wire::source_control_snapshot::BranchState::Operation(value)) = &snapshot.branch_state
    else {
        panic!("expected merge/rebase: {snapshot:?}");
    };
    value
}

// B-001, B-002: 表示に使う一覧を公開購読で確かめる。
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_変更の区画と状態と行数を配信し名前で絞る() {
    // Given
    let f = Fixture::new().await;
    f.commit("src/bridge.js", "old\n", "bridge");
    f.commit("src/deleted.txt", "removed\n", "deleted");
    f.commit("FileTabPane.swift", "before\n", "swift");
    f.write("src/bridge.js", "new\nextra\n");
    f.write("src/added.txt", "added\n");
    git(&f.repo, &["add", "src/bridge.js", "src/added.txt"]);
    std::fs::remove_file(f.repo.join("src/deleted.txt")).unwrap();
    f.write("FileTabPane.swift", "after\n");
    f.write("thread-card.css", "untracked\n");
    // When
    let s = f.snapshot(None).await;
    // Then
    assert_eq!(s.changed_file_count, 5);
    assert_eq!((s.staged.len(), s.unstaged.len()), (2, 3));
    let rows: Vec<_> = s
        .staged
        .iter()
        .chain(&s.unstaged)
        .map(|r| {
            (
                r.path.as_str(),
                r.name.as_str(),
                r.directory.as_str(),
                r.kind,
                r.additions,
                r.deletions,
            )
        })
        .collect();
    use wire::SourceControlChangeKind as K;
    assert_eq!(
        rows,
        vec![
            ("src/added.txt", "added.txt", "src", K::Added as i32, 1, 0),
            (
                "src/bridge.js",
                "bridge.js",
                "src",
                K::Modified as i32,
                2,
                1
            ),
            (
                "FileTabPane.swift",
                "FileTabPane.swift",
                "",
                K::Modified as i32,
                1,
                1
            ),
            (
                "src/deleted.txt",
                "deleted.txt",
                "src",
                K::Deleted as i32,
                0,
                1
            ),
            (
                "thread-card.css",
                "thread-card.css",
                "",
                K::Untracked as i32,
                1,
                0
            ),
        ]
    );
    // When
    let filtered = f.snapshot(Some("bridge")).await;
    // Then
    assert_eq!(
        filtered
            .staged
            .iter()
            .chain(&filtered.unstaged)
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["bridge.js"]
    );
}

// B-006, B-008, B-018, B-019
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_stageとunstageとcommitを一覧とgitから観測する() {
    // Given
    let f = Fixture::new().await;
    f.commit("file.txt", "base\n", "base");
    git(&f.repo, &["push"]);
    f.write("file.txt", "staged\n");
    // When
    let s = f.snapshot(None).await;
    // Then
    assert!(s.staged.is_empty());
    assert!(!s.can_commit);
    // When
    f.stage(&["file.txt"]).await;
    // Then
    let s = f.snapshot(None).await;
    assert_eq!(s.staged[0].path, "file.txt");
    assert!(s.can_commit);
    assert!(s.unstaged.is_empty());
    // When
    f.socket
        .git
        .git_commit(rpc::GitCommitRequest {
            worktree_path: f.path(),
            message: "acceptance commit".into(),
            follow_up: rpc::GitCommitFollowUp::None.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    let s = f.snapshot(None).await;
    assert!(s.staged.is_empty());
    assert!(!s.can_commit);
    assert_eq!(tracking(&s).ahead, 1);
    assert_eq!(
        git(&f.repo, &["log", "-1", "--format=%B"]),
        "acceptance commit"
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:file.txt"]), "staged");
    // Given
    f.write("file.txt", "unstage\n");
    f.stage(&["file.txt"]).await;
    // When
    f.unstage(&["file.txt"]).await;
    // Then
    let s = f.snapshot(None).await;
    assert!(s.staged.is_empty());
    assert_eq!(s.unstaged[0].path, "file.txt");
    // Given
    f.write("second.txt", "second\n");
    f.write("third.txt", "third\n");
    // When
    f.stage(&[]).await;
    // Then
    let s = f.snapshot(None).await;
    assert_eq!(s.staged.len(), 3);
    assert!(s.unstaged.is_empty());
    // When
    f.unstage(&[]).await;
    // Then
    let s = f.snapshot(None).await;
    assert_eq!(s.unstaged.len(), 3);
    assert!(s.staged.is_empty());
}

// B-017, B-021: stage 済みの内容を残して、まだ stage していない内容だけを破棄する。
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_同じファイルの区画別行数と破棄後のstage済み変更を保つ() {
    // Given
    let f = Fixture::new().await;
    f.commit("bridge.js", "base\n", "base");
    f.write("bridge.js", "staged\nextra\n");
    f.stage(&["bridge.js"]).await;
    f.write("bridge.js", "unstaged\n");
    // When
    let s = f.snapshot(None).await;
    // Then
    assert_eq!(
        (
            s.staged[0].path.as_str(),
            s.staged[0].additions,
            s.staged[0].deletions
        ),
        ("bridge.js", 2, 1)
    );
    assert_eq!(
        (
            s.unstaged[0].path.as_str(),
            s.unstaged[0].additions,
            s.unstaged[0].deletions
        ),
        ("bridge.js", 1, 2)
    );
    // When
    f.socket
        .git
        .git_discard(rpc::GitDiscardRequest {
            worktree_path: f.path(),
            paths: vec!["bridge.js".into()],
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    let after = f.snapshot(None).await;
    assert_eq!(after.staged, s.staged);
    assert!(after.unstaged.is_empty());
    assert_eq!(
        std::fs::read_to_string(f.repo.join("bridge.js")).unwrap(),
        "staged\nextra\n"
    );
}

// B-004, B-005, B-010, B-015, B-024
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_先行commitをpublishしてupstreamを配信する() {
    // Given
    let f = Fixture::new().await;
    git(&f.repo, &["checkout", "-b", "feature/thread-ui"]);
    f.commit("one.txt", "one\n", "one");
    let head = f.commit("two.txt", "two\n", "two");
    // When
    let s = f.snapshot(None).await;
    // Then
    assert_eq!(s.branch, "feature/thread-ui");
    let Some(wire::source_control_snapshot::BranchState::Unpublished(value)) = s.branch_state
    else {
        panic!("expected unpublished");
    };
    assert_eq!(value.unpushed_commits, 2);
    // When
    f.push(false).await;
    // Then
    assert_eq!(git(&f.origin, &["rev-parse", "feature/thread-ui"]), head);
    let s = f.snapshot(None).await;
    assert_eq!(
        (
            tracking(&s).upstream.as_str(),
            tracking(&s).ahead,
            tracking(&s).behind
        ),
        ("origin/feature/thread-ui", 0, 0)
    );
    // Given
    f.commit("one.txt", "one changed\n", "third");
    let head = f.commit("two.txt", "two changed\n", "fourth");
    let s = f.snapshot(None).await;
    assert_eq!((tracking(&s).ahead, tracking(&s).behind), (2, 0));
    assert_eq!(s.changed_file_count, 0);
    // When
    f.push(false).await;
    // Then
    assert_eq!(git(&f.origin, &["rev-parse", "feature/thread-ui"]), head);
    assert_eq!(tracking(&f.snapshot(None).await).ahead, 0);
}

// B-007
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_commitの署名はgitの設定に従う() {
    // Given
    let f = Fixture::new().await;
    let key = f.root.path().join("signing-key");
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .status()
        .unwrap();
    assert!(status.success());
    git(&f.repo, &["config", "gpg.format", "ssh"]);
    git(
        &f.repo,
        &["config", "user.signingkey", key.to_str().unwrap()],
    );
    git(&f.repo, &["config", "commit.gpgsign", "true"]);
    f.write("signed.txt", "signed\n");
    f.stage(&["signed.txt"]).await;
    // When
    f.socket
        .git
        .git_commit(rpc::GitCommitRequest {
            worktree_path: f.path(),
            message: "signed commit".into(),
            follow_up: rpc::GitCommitFollowUp::None.into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    assert!(git(&f.repo, &["cat-file", "commit", "HEAD"])
        .contains("gpgsig -----BEGIN SSH SIGNATURE-----"));
}

// B-003: 確認後の Force Push の公開 API。
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_force_pushでupstreamをローカルcommitに揃える() {
    // Given
    let f = Fixture::new().await;
    git(&f.repo, &["remote", "rename", "origin", "upstream"]);
    f.commit("file.txt", "before\n", "before");
    git(&f.repo, &["push"]);
    git(&f.repo, &["commit", "--amend", "-m", "rewritten"]);
    let head = git(&f.repo, &["rev-parse", "HEAD"]);
    // When
    f.push(true).await;
    // Then
    assert_eq!(git(&f.origin, &["rev-parse", "main"]), head);
}

// B-011, B-012, B-013, B-014
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_fetchとpullとsyncとcommit後の送信を行う() {
    // Given
    let f = Fixture::new().await;
    git(&f.repo, &["remote", "rename", "origin", "upstream"]);
    git(&f.repo, &["checkout", "-b", "remote-work"]);
    let remote = f.commit("remote.txt", "remote\n", "remote");
    git(&f.repo, &["push", "upstream", "HEAD:main"]);
    git(&f.repo, &["checkout", "main"]);
    // push が更新した remote-tracking ref を、最後の取得時点に戻す。
    git(
        &f.repo,
        &["update-ref", "refs/remotes/upstream/main", "main"],
    );
    let before = git(&f.repo, &["rev-parse", "HEAD"]);
    f.write("local.txt", "local\n");
    // When
    f.socket
        .git
        .git_fetch(rpc::GitFetchRequest {
            worktree_path: f.path(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    assert_eq!(tracking(&f.snapshot(None).await).behind, 1);
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), before);
    assert_eq!(
        std::fs::read_to_string(f.repo.join("local.txt")).unwrap(),
        "local\n"
    );
    assert!(!f.repo.join("remote.txt").exists());
    // When
    f.socket
        .git
        .git_pull(rpc::GitPullRequest {
            worktree_path: f.path(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), remote);
    assert_eq!(tracking(&f.snapshot(None).await).behind, 0);
    // Given
    git(&f.repo, &["checkout", "remote-work"]);
    let remote = f.commit("remote.txt", "second remote\n", "second remote");
    git(&f.repo, &["push", "upstream", "HEAD:main"]);
    git(&f.repo, &["checkout", "main"]);
    let local = f.commit("local.txt", "local\n", "local");
    // When
    f.socket
        .git
        .git_sync(rpc::GitSyncRequest {
            worktree_path: f.path(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    git(&f.repo, &["merge-base", "--is-ancestor", &remote, "HEAD"]);
    git(&f.repo, &["merge-base", "--is-ancestor", &local, "HEAD"]);
    assert_eq!(
        git(&f.origin, &["rev-parse", "main"]),
        git(&f.repo, &["rev-parse", "HEAD"])
    );
    let s = f.snapshot(None).await;
    assert_eq!((tracking(&s).ahead, tracking(&s).behind), (0, 0));
    for follow_up in [rpc::GitCommitFollowUp::Push, rpc::GitCommitFollowUp::Sync] {
        // Given
        f.write("local.txt", &format!("{follow_up:?}\n"));
        f.stage(&["local.txt"]).await;
        // When
        f.socket
            .git
            .git_commit(rpc::GitCommitRequest {
                worktree_path: f.path(),
                message: format!("{follow_up:?}"),
                follow_up: follow_up.into(),
                ..Default::default()
            })
            .await
            .unwrap();
        // Then
        assert_eq!(
            git(&f.origin, &["rev-parse", "main"]),
            git(&f.repo, &["rev-parse", "HEAD"])
        );
        assert_eq!(
            git(&f.repo, &["show", "HEAD:local.txt"]),
            format!("{follow_up:?}")
        );
        assert_eq!(tracking(&f.snapshot(None).await).ahead, 0);
    }
}

// B-016, B-025, B-026, B-028, B-029, B-030, B-031
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_pullのconflictを配信し解決して続行または中止する() {
    for rebase in [false, true] {
        for abort in [false, true] {
            // Given
            let f = Fixture::new().await;
            f.commit("bridge.js", "base\n", "base");
            git(&f.repo, &["push"]);
            git(&f.repo, &["checkout", "-b", "remote-work"]);
            f.commit("bridge.js", "remote\n", "remote");
            git(&f.repo, &["push", "origin", "HEAD:main"]);
            git(&f.repo, &["checkout", "main"]);
            let before = f.commit("bridge.js", "local\n", "local");
            git(
                &f.repo,
                &[
                    "config",
                    "pull.rebase",
                    if rebase { "true" } else { "false" },
                ],
            );
            let error = f
                .socket
                .git
                .git_push(rpc::GitPushRequest {
                    worktree_path: f.path(),
                    ..Default::default()
                })
                .await
                .unwrap_err();
            assert!(error.message.as_ref().is_some_and(|m| !m.is_empty()));
            // When
            let error = f
                .socket
                .git
                .git_pull(rpc::GitPullRequest {
                    worktree_path: f.path(),
                    ..Default::default()
                })
                .await
                .unwrap_err();
            // Then
            assert!(error.message.as_ref().is_some_and(|m| !m.is_empty()));
            let s = f.snapshot(None).await;
            assert_eq!(s.conflicts.len(), 1);
            assert_eq!(s.conflicts[0].path, "bridge.js");
            assert_eq!(operation(&s).conflict_count, 1);
            assert!(!operation(&s).can_continue);
            assert!(!s.can_commit);
            if abort {
                // When
                f.socket
                    .git
                    .git_abort_operation(rpc::GitAbortOperationRequest {
                        worktree_path: f.path(),
                        ..Default::default()
                    })
                    .await
                    .unwrap();
                // Then
                assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), before);
                assert_eq!(
                    std::fs::read_to_string(f.repo.join("bridge.js")).unwrap(),
                    "local\n"
                );
                let s = f.snapshot(None).await;
                assert!(s.conflicts.is_empty());
                assert!(!matches!(
                    s.branch_state,
                    Some(wire::source_control_snapshot::BranchState::Operation(_))
                ));
            } else {
                // Given
                f.write("bridge.js", "resolved\n");
                // When
                f.stage(&["bridge.js"]).await;
                // Then
                let s = f.snapshot(None).await;
                assert!(s.conflicts.is_empty());
                assert_eq!(s.staged[0].path, "bridge.js");
                assert!(operation(&s).can_continue);
                assert!(!s.can_commit);
                assert!(operation(&s)
                    .message
                    .as_ref()
                    .is_some_and(|message| !message.is_empty()));
                // When
                f.socket
                    .git
                    .git_continue_operation(rpc::GitContinueOperationRequest {
                        worktree_path: f.path(),
                        message: "resolved message".into(),
                        ..Default::default()
                    })
                    .await
                    .unwrap();
                // Then
                assert_eq!(
                    git(&f.repo, &["log", "-1", "--format=%B"]),
                    "resolved message"
                );
                if !rebase {
                    assert_eq!(
                        git(&f.repo, &["rev-list", "--parents", "-n", "1", "HEAD"])
                            .split_whitespace()
                            .count(),
                        3
                    );
                }
                assert!(!matches!(
                    f.snapshot(None).await.branch_state,
                    Some(wire::source_control_snapshot::BranchState::Operation(_))
                ));
            }
        }
    }
}

// B-025, B-027
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_rebaseの進捗と次のconflictと最終commitを配信する() {
    // Given
    let f = Fixture::new().await;
    f.commit("src/bridge.js", "base\n", "base bridge");
    f.commit("src/other.js", "base\n", "base other");
    git(&f.repo, &["checkout", "-b", "feature/thread-ui"]);
    f.commit("first.txt", "first\n", "first");
    f.write("src/bridge.js", "second\n");
    f.write("src/other.js", "second\n");
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-m", "second"]);
    f.commit("src/bridge.js", "third\n", "third");
    git(&f.repo, &["checkout", "main"]);
    f.commit("src/bridge.js", "main\n", "main bridge");
    f.commit("src/other.js", "main\n", "main other");
    git(&f.repo, &["checkout", "feature/thread-ui"]);
    let result = Command::new("git")
        .args(["rebase", "main"])
        .current_dir(&f.repo)
        .output()
        .unwrap();
    assert!(
        !result.status.success(),
        "expected conflict while preparing rebase"
    );
    // When
    let s = f.snapshot(None).await;
    // Then
    assert_eq!(s.branch, "feature/thread-ui");
    let op = operation(&s);
    let Some(wire::source_control_operation::Kind::Rebase(progress)) = &op.kind else {
        panic!("expected rebase");
    };
    assert_eq!(
        (progress.onto.as_str(), progress.step, progress.total),
        ("main", 2, 3)
    );
    assert_eq!(op.conflict_count, 2);
    assert_eq!(op.message.as_deref().map(str::trim), Some("second"));
    assert!(!op.can_continue);
    assert_eq!(
        s.conflicts
            .iter()
            .map(|r| (r.name.as_str(), r.directory.as_str(), r.kind))
            .collect::<Vec<_>>(),
        vec![
            (
                "bridge.js",
                "src",
                wire::SourceControlConflictKind::BothModified as i32
            ),
            (
                "other.js",
                "src",
                wire::SourceControlConflictKind::BothModified as i32
            ),
        ]
    );
    // Given
    f.write("src/bridge.js", "resolved second\n");
    f.write("src/other.js", "resolved second\n");
    f.stage(&["src/bridge.js", "src/other.js"]).await;
    // When
    let error = f
        .socket
        .git
        .git_continue_operation(rpc::GitContinueOperationRequest {
            worktree_path: f.path(),
            message: "rewritten second".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    // Then
    assert!(error.message.as_ref().is_some_and(|m| !m.is_empty()));
    let s = f.snapshot(None).await;
    let op = operation(&s);
    let Some(wire::source_control_operation::Kind::Rebase(progress)) = &op.kind else {
        panic!("expected rebase");
    };
    assert_eq!((progress.step, progress.total), (3, 3));
    assert_eq!(op.conflict_count, 1);
    assert_eq!(
        s.conflicts
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        ["src/bridge.js"]
    );
    // Given
    f.write("src/bridge.js", "resolved third\n");
    f.stage(&["src/bridge.js"]).await;
    // When
    f.socket
        .git
        .git_continue_operation(rpc::GitContinueOperationRequest {
            worktree_path: f.path(),
            message: "rewritten third".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    assert_eq!(
        git(&f.repo, &["log", "-1", "--format=%B"]),
        "rewritten third"
    );
    assert!(!matches!(
        f.snapshot(None).await.branch_state,
        Some(wire::source_control_snapshot::BranchState::Operation(_))
    ));
}

// B-022, B-023: macOS の実際のゴミ箱への移動を確認する。
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_未追跡ファイルをゴミ箱に移して区画全体を破棄する() {
    // Given
    let f = Fixture::new().await;
    let trash = dirs::home_dir().unwrap().join(".Trash");
    for all in [false, true] {
        let name = format!("thread-card-{}.css", uuid::Uuid::new_v4());
        let destination = trash.join(&name);
        f.write(&name, "recoverable\n");
        let mut paths = vec![name.clone()];
        if all {
            f.commit("first.txt", "before\n", "first");
            f.commit("second.txt", "before\n", "second");
            f.write("first.txt", "after\nextra\n");
            f.write("second.txt", "after\n");
            paths.extend(["first.txt".into(), "second.txt".into()]);
        }
        // When
        f.socket
            .git
            .git_discard(rpc::GitDiscardRequest {
                worktree_path: f.path(),
                paths,
                ..Default::default()
            })
            .await
            .unwrap();
        // Then
        assert!(!f.repo.join(&name).exists());
        let content = std::fs::read_to_string(&destination);
        if content.is_ok() {
            std::fs::remove_file(&destination).unwrap();
        }
        assert_eq!(content.unwrap(), "recoverable\n");
        assert!(f.snapshot(None).await.unstaged.is_empty());
        if all {
            assert_eq!(
                std::fs::read_to_string(f.repo.join("first.txt")).unwrap(),
                "before\n"
            );
            assert_eq!(
                std::fs::read_to_string(f.repo.join("second.txt")).unwrap(),
                "before\n"
            );
        }
    }
}

// B-009
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_先行が2でstage済み変更があるとcommit後の先行数は3になる() {
    // Given
    let f = Fixture::new().await;
    f.commit("first.txt", "first\n", "first");
    f.commit("second.txt", "second\n", "second");
    f.write("first.txt", "pending\n");
    git(&f.repo, &["add", "first.txt"]);
    // When
    let snapshot = f.snapshot(None).await;
    // Then
    let tracking = tracking(&snapshot);
    assert_eq!(
        (tracking.ahead, tracking.behind, tracking.ahead_after_commit),
        (2, 0, 3)
    );
}

// B-023
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_区画全体の破棄で消える行数とゴミ箱へ移す件数を配信する() {
    // Given
    let f = Fixture::new().await;
    let lines = |prefix: &str, count: usize| {
        (0..count)
            .map(|n| format!("{prefix}{n}\n"))
            .collect::<String>()
    };
    f.commit("bridge.js", &lines("old", 9), "bridge");
    f.commit("other.js", &lines("before", 2), "other");
    f.write("bridge.js", &lines("new", 23));
    f.write("other.js", &lines("after", 4));
    f.write("thread-card.css", &lines("untracked", 7));
    // When
    let snapshot = f.snapshot(None).await;
    // Then
    let summary = snapshot.discard_all.expect("discard summary");
    assert_eq!(
        (summary.additions, summary.deletions, summary.trashed_files),
        (27, 11, 1)
    );
}

// B-032
#[tokio::test(flavor = "multi_thread")]
async fn test_source_control_breakで停止したrebaseをcommitを増やさず続行する() {
    // Given
    let f = Fixture::new().await;
    f.commit("file.txt", "change\n", "change");
    let result = Command::new("git")
        .args(["rebase", "-i", "HEAD~1"])
        .env(
            "GIT_SEQUENCE_EDITOR",
            "sed -i.bak -e '$a\\
break\n'",
        )
        .current_dir(&f.repo)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let before = git(&f.repo, &["rev-list", "--count", "HEAD"]);
    // When
    let snapshot = f.snapshot(None).await;
    // Then
    let op = operation(&snapshot);
    assert!(matches!(
        op.kind,
        Some(wire::source_control_operation::Kind::Rebase(_))
    ));
    assert!(op.message.is_none());
    assert!(op.can_continue);
    // When
    f.socket
        .git
        .git_continue_operation(rpc::GitContinueOperationRequest {
            worktree_path: f.path(),
            message: String::new(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    assert_eq!(git(&f.repo, &["rev-list", "--count", "HEAD"]), before);
    assert!(!matches!(
        f.snapshot(None).await.branch_state,
        Some(wire::source_control_snapshot::BranchState::Operation(_))
    ));
}
