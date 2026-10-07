#![cfg(unix)]
use releash_sdk::{daemon, discovery};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

fn executable(directory: &std::path::Path, body: &str) -> std::path::PathBuf {
    let path = directory.join("releashd");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[tokio::test]
async fn test_独立起動_終了状態とstderrの末尾を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("logs")).unwrap();
    std::fs::write(
        directory.path().join("logs/daemon-startup.stderr"),
        "previous launch",
    )
    .unwrap();
    let executable = executable(directory.path(), "printf 'startup failure' >&2; exit 7");
    // When
    let error = daemon::start(&executable, directory.path(), directory.path())
        .await
        .unwrap_err();
    // Then
    let daemon::DaemonError::Exited { status, stderr } = error else {
        panic!("{error}")
    };
    assert_eq!(status.code(), Some(7));
    assert_eq!(stderr, "startup failure");
}

#[tokio::test]
async fn test_独立起動_別sessionでcwdと明示data_dirを使いstdinが閉じても動き続ける() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path().join("cwd");
    std::fs::create_dir(&cwd).unwrap();
    let executable = executable(
        directory.path(),
        "printf '%s' $$ > \"$2/pid\"; pwd > \"$2/observed-cwd\"; read -r line; exec /bin/sleep 60",
    );
    let data_dir = directory.path().to_path_buf();
    let task = tokio::spawn(async move { daemon::start(&executable, &data_dir, &cwd).await });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !directory.path().join("pid").exists() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let pid: u32 = std::fs::read_to_string(directory.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    let started = discovery::process_start_time(pid).unwrap();
    let discovery = discovery::LocalApiDiscovery {
        port: 12345,
        token: "test".into(),
        instance_id: "test".into(),
        pid,
        process_started_at: started,
    };
    std::fs::write(
        directory.path().join("client-api.json"),
        serde_json::to_vec(&discovery).unwrap(),
    )
    .unwrap();
    // When
    let result = task.await.unwrap().unwrap();
    // Then
    assert_eq!(result, discovery);
    assert_eq!(unsafe { libc::getsid(pid as i32) }, pid as i32);
    assert_ne!(unsafe { libc::getsid(0) }, pid as i32);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("observed-cwd"))
            .unwrap()
            .trim(),
        directory
            .path()
            .join("cwd")
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
    );
    assert_eq!(unsafe { libc::kill(pid as i32, 0) }, 0);
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGTERM) }, 0);
}

#[tokio::test]
async fn test_独立起動_上限を超えてもプロセスを止めずstderrを返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let executable = executable(
        directory.path(),
        "printf '%s' $$ > \"$2/pid\"; printf 'still starting' >&2; exec /bin/sleep 60",
    );
    // When
    let result = daemon::start(&executable, directory.path(), directory.path()).await;
    // Then
    assert!(
        matches!(result, Err(daemon::DaemonError::StartupTimeout { ref stderr }) if stderr == "still starting")
    );
    let pid: i32 = std::fs::read_to_string(directory.path().join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
}

#[tokio::test]
async fn test_停止未完了_上限で失敗を返し生きているサーバを強制終了しない() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    // Given
    let directory = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let discovery = discovery::LocalApiDiscovery {
        port: listener.local_addr().unwrap().port(),
        token: "test".into(),
        instance_id: "test".into(),
        pid: std::process::id(),
        process_started_at: discovery::process_start_time(std::process::id()).unwrap(),
    };
    std::fs::write(
        directory.path().join("client-api.json"),
        serde_json::to_vec(&discovery).unwrap(),
    )
    .unwrap();
    let info = prost::Message::encode_to_vec(&releash_sdk::wire::ServerInfo {
        daemon_id: discovery.instance_id.clone(),
        pid: discovery.pid,
        process_started_at: discovery.process_started_at,
        ..Default::default()
    });
    let stops = Arc::new(AtomicUsize::new(0));
    let counted = stops.clone();
    let router = axum::Router::new()
        .route(
            "/releash.client.v1.ClientService/GetServerInfo",
            axum::routing::post(move || {
                let info = info.clone();
                async move { ([("content-type", "application/proto")], info) }
            }),
        )
        .route(
            "/releash.client.v1.ClientService/StopDaemon",
            axum::routing::post(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                async { ([("content-type", "application/proto")], Vec::<u8>::new()) }
            }),
        );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    // When
    let result = daemon::stop(directory.path(), &discovery).await;
    // Then
    assert!(matches!(result, Err(daemon::DaemonError::ShutdownTimeout)));
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    assert_eq!(daemon::running(directory.path()).unwrap(), Some(discovery));
    server.abort();
}
