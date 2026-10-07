use releashd::test_support::integration::daemon::compose;
use releashd::test_support::integration::daemon::migrate_legacy_execution_archives;
use releashd::test_support::integration::daemon::shutdown_with_deadline;
use releashd::test_support::integration::daemon::Daemon;
use releashd::test_support::integration::platform::FakeShutdown;
use releashd::test_support::integration::platform::STAGES;
use std::sync::Arc;

#[tokio::test]
pub async fn test_daemon終了_受信先が閉じた場合のエラーを維持する() {
    // Given
    let (sender, exit) = tokio::sync::mpsc::channel(1);
    drop(sender);
    let shutdown = Arc::new(releashd::test_support::integration::platform::FakeShutdown::default());
    // When
    let directory = tempfile::tempdir().unwrap();
    let server =
        releashd::test_support::integration::transport::test_binding(directory.path().into())
            .unwrap()
            .start(axum::Router::new(), &tokio::runtime::Handle::current())
            .unwrap();
    let error = Daemon::test_new(
        shutdown.clone(),
        server,
        exit,
        releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(
            releashd::test_support::integration::daemon::serving(),
        ),
    )
    .wait()
    .await
    .unwrap_err();
    // Then
    assert_eq!(error, "daemon exit channel closed");
    assert!(shutdown.calls.lock().unwrap().is_empty());
}

#[test]
pub fn test_daemon終了_成功と各段階の失敗と停止で完了通知と指定codeを返す() {
    for scenario in std::iter::once("success".to_string())
        .chain(
            ["fail", "block"]
                .into_iter()
                .flat_map(|mode| STAGES.map(|stage| format!("{mode}:{stage}"))),
        )
        .chain(["stop", "drain", "flush"].map(|stage| format!("terminal:block:{stage}")))
        .chain(["command:admission".to_string()])
    {
        // Given
        let directory = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "adaptor_controller_daemon::test_daemon終了_subprocess",
                "--ignored",
                "--nocapture",
            ])
            .env("RELEASH_SHUTDOWN_TEST_CASE", &scenario)
            .env("RELEASH_SHUTDOWN_DATA_DIR", directory.path())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        // When
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > std::time::Duration::from_secs(10) {
                child.kill().unwrap();
                let output = child.wait_with_output().unwrap();
                panic!("{scenario}: daemon did not exit: {output:?}");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        // Then
        assert_eq!(
            output.status.code(),
            Some(23),
            "{scenario}: {stdout}\n{stderr}"
        );
        assert_eq!(
            stdout.matches("releash-shutdown-complete").count(),
            1,
            "{scenario}: {stdout}"
        );
        let called: Vec<_> = stdout
            .lines()
            .filter_map(|line| line.strip_prefix("shutdown-stage:"))
            .collect();
        for name in ["client-api.json"] {
            assert!(
                !directory.path().join(name).exists(),
                "{scenario}: {name} remains"
            );
        }
        if scenario == "command:admission" {
            assert!(
                stdout.contains("15 second deadline exceeded; exiting"),
                "{stdout}"
            );
        } else if let Some(blocked) = scenario.strip_prefix("terminal:block:") {
            assert!(
                stdout.contains(&format!("terminal-blocked:{blocked}")),
                "{stdout}"
            );
            assert!(
                stdout.contains("15 second deadline exceeded; exiting"),
                "{stdout}"
            );
        } else if let Some(blocked) = scenario.strip_prefix("block:") {
            let index = STAGES.iter().position(|stage| *stage == blocked).unwrap();
            assert_eq!(called, STAGES[..=index]);

            assert!(stdout.contains("15 second deadline exceeded; exiting"));
        } else {
            assert_eq!(called, STAGES);
            assert!(!stdout.contains("shutdown-deadline:"));
            if let Some(failed) = scenario.strip_prefix("fail:") {
                assert!(
                    stdout.contains(&format!("failed: {failed} failed")),
                    "{stdout}"
                );
            }
        }
    }
}

#[test]
#[ignore = "invoked by the daemon exit subprocess test"]
pub fn test_daemon終了_subprocess() {
    struct Logger;
    impl log::Log for Logger {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            println!("{}", record.args());
        }
        fn flush(&self) {
            panic!("shutdown must not flush the local logger");
        }
    }
    log::set_logger(&Logger).unwrap();
    log::set_max_level(log::LevelFilter::Error);
    let scenario = std::env::var("RELEASH_SHUTDOWN_TEST_CASE").unwrap();
    let mut shutdown = releashd::test_support::integration::platform::FakeShutdown {
        delay: std::time::Duration::from_secs(2),
        ..Default::default()
    };
    for stage in releashd::test_support::integration::platform::STAGES {
        if scenario == format!("fail:{stage}") {
            shutdown.failed = Some(stage);
        }
        if scenario == format!("block:{stage}") {
            shutdown.blocked = Some(stage);
        }
    }
    let (sender, exit) = tokio::sync::mpsc::channel(1);
    sender.try_send(23).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    let error = runtime.block_on(async {
        let directory = std::path::PathBuf::from(std::env::var_os("RELEASH_SHUTDOWN_DATA_DIR").unwrap());
        let server = releashd::test_support::integration::transport::test_binding(directory).unwrap()
            .start(axum::Router::new(), &tokio::runtime::Handle::current()).unwrap();
        server.publish_discovery().unwrap();
        if scenario == "command:admission" || scenario.starts_with("terminal:block:") {
            let blocked = scenario.strip_prefix("terminal:block:").unwrap_or("stop");
            let blocked = match blocked {
                "stop" => "stop",
                "drain" => "drain",
                "flush" => "flush",
                _ => unreachable!(),
            };
            let fixture = crate::adaptor_gateway_workflow_workflow_host_test_helpers::Fixture::new(0);
            let mut pty = releashd::test_support::integration::terminal::FakePtyGateway::new();
            let owner = releashd::test_support::integration::terminal::TerminalSurfaceOwner::session(
                releashd::test_support::integration::workspace::WorkspaceIdentity::new("/repo"), "session",
            ).unwrap();
            pty.shutdown_surfaces.push(
                releashd::test_support::integration::terminal::TerminalSurface::with_checkpoint(
                    1,
                    owner,
                    None,
                    releashd::test_support::integration::terminal::TerminalSurfaceCheckpoint::empty(80, 24),
                ),
            );
            let (started, ready) = tokio::sync::oneshot::channel();
            let (_release, receiver) = std::sync::mpsc::channel();
            *pty.shutdown_gate.lock() = Some((blocked, started, receiver));
            let hub = Arc::new(releashd::test_support::integration::platform::TerminalSurfaceEventHub::new());
            let terminal = Arc::new(releashd::test_support::integration::terminal::TerminalSurfaceApplication::new(std::sync::Arc::new(releashd::test_support::integration::telemetry::TelemetryGateway),
                Arc::new(pty),
                Arc::new(releashd::test_support::integration::terminal::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
                hub,
            ));
            let shutdown = releashd::test_support::integration::platform::DaemonShutdownGateway {
                workflow: Arc::new(releashd::test_support::integration::workflow::WorkflowRuntimeUsecase::new(Arc::new(
                    releashd::test_support::integration::workflow::WorkflowRuntimeCommandGateway::new_with_driver(
                        fixture.app.clone(), Arc::new(fixture.host.clone()),
                    ),
                ), Arc::new(releashd::test_support::integration::workflow::ExecutionTreeArchiveFactRepository::from_backend(releashd::test_support::integration::workflow::FactLogReadBackend::Live(fixture.app.store.clone().unwrap()))))),
                terminal,
                server: server.clone(),
                stop_observer: Arc::new(|| {}),
                telemetry: parking_lot::Mutex::new(None),
            };
            let repository = fixture.daemon_repository();
            let daemon = releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(repository.clone());
            daemon.stop(releashd::test_support::integration::daemon::StopRequest::Exit { code: 23 }).await;
            let _admission = if scenario == "command:admission" {
                Some(repository.admission().await)
            } else { None };
            if scenario != "command:admission" { tokio::spawn(async move {
                ready.await.unwrap();
                assert!(sender.is_closed());
                for code in 0..1000 {
                    assert!(sender.try_send(code).is_err());
                }
                tokio::time::advance(std::time::Duration::from_secs(15)).await;
            }); }
            Daemon::test_new(Arc::new(shutdown), server, exit, daemon).wait().await
        } else {
            Daemon::test_new(Arc::new(shutdown), server, exit, releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(releashd::test_support::integration::daemon::serving())).wait().await
        }
    }).unwrap_err();
    panic!("daemon wait returned: {error}");
}

#[tokio::test]
pub async fn test_daemon起動のarchive移行結線_未終了対象をabortし事実保存後に旧ファイルを削除する()
{
    use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture;
    use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_workflow;
    use releashd::test_support::integration::workflow::ExecutionStatus;
    use releashd::test_support::integration::workflow::ExecutionTreeArchiveRepository;
    // Given
    let fixture = archive_fixture();
    let id = archive_workflow(&fixture).await;
    let path = fixture
        .directory
        .path()
        .join("workflow_execution_archives.json");
    std::fs::write(&path, serde_json::json!({"executions": {id.clone(): {"archivedAt": 12.345678, "archiveReason": "manual"}}}).to_string()).unwrap();
    // When
    migrate_legacy_execution_archives(
        fixture.directory.path(),
        fixture.store.clone(),
        &fixture.runtime,
    )
    .await
    .unwrap();
    // Then
    assert!(!path.exists());
    assert_eq!(
        fixture.repository.target(&id).await.unwrap().status,
        ExecutionStatus::Aborted
    );
    let archived = fixture
        .repository
        .archive_snapshot_for(&[id])
        .await
        .unwrap()
        .records
        .remove(0);
    assert_eq!(archived.archived_at, 12.345678);
    assert_eq!(archived.archive_reason, "manual");
}

#[tokio::test]
pub async fn test_開始計測_組み立て失敗前に起点を記録する() {
    // Given
    let _guard = releashd::test_support::integration::telemetry::lock_test_telemetry();
    releashd::test_support::integration::telemetry::reset_test_metrics();
    releashd::test_support::integration::telemetry::set_performance_configured(true);
    let directory = tempfile::tempdir().unwrap();
    let invalid_data_dir = directory.path().join("file");
    std::fs::write(&invalid_data_dir, "not a directory").unwrap();

    // When
    let result = compose(
        invalid_data_dir,
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        Ok(std::ffi::OsString::new()),
    )
    .await;
    releashd::test_support::integration::telemetry::record_first_repo_snapshot_ready();
    releashd::test_support::integration::telemetry::record_first_repo_snapshot_ready();

    // Then
    assert!(result.is_err());
    let records = releashd::test_support::integration::telemetry::test_metric_records();
    let startup: Vec<_> = records
        .iter()
        .filter(|record| record.name == "releash.startup.duration_ms")
        .collect();
    assert_eq!(startup.len(), 1);
    assert!(startup[0].value >= 0.0);
    releashd::test_support::integration::telemetry::reset_test_metrics();
}

#[tokio::test(start_paused = true)]
pub async fn test_終了処理_どの段階が停止しても全体で15秒以内に打ち切る() {
    for (index, blocked) in STAGES.into_iter().enumerate() {
        // Given
        let gateway = FakeShutdown {
            blocked: Some(blocked),
            delay: std::time::Duration::from_secs(2),
            ..Default::default()
        };
        let directory = tempfile::tempdir().unwrap();
        let server =
            releashd::test_support::integration::transport::test_binding(directory.path().into())
                .unwrap()
                .start(axum::Router::new(), &tokio::runtime::Handle::current())
                .unwrap();
        server.publish_discovery().unwrap();
        let started = tokio::time::Instant::now();
        // When
        releashd::test_support::integration::daemon::shutdown_with_deadline(&gateway, &server)
            .await;
        // Then
        assert_eq!(started.elapsed(), std::time::Duration::from_secs(15));
        assert_eq!(*gateway.calls.lock().unwrap(), STAGES[..=index]);
        for name in ["client-api.json"] {
            assert!(!directory.path().join(name).exists());
        }
    }
}

#[tokio::test(start_paused = true)]
pub async fn test_終了処理_期限切れで別サーバの発見ファイルを削除しない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let start = || {
        releashd::test_support::integration::transport::test_binding(directory.path().into())
            .unwrap()
            .start(axum::Router::new(), &tokio::runtime::Handle::current())
            .unwrap()
    };
    let original = start();
    original.publish_discovery().unwrap();
    let replacement = start();
    replacement.publish_discovery().unwrap();
    let files = ["client-api.json"].map(|name| directory.path().join(name));
    let contents = files.each_ref().map(|path| std::fs::read(path).unwrap());
    let gateway = FakeShutdown {
        blocked: Some("commands"),
        ..Default::default()
    };
    // When
    shutdown_with_deadline(&gateway, &original).await;
    // Then
    assert_eq!(
        files.each_ref().map(|path| std::fs::read(path).unwrap()),
        contents
    );
}
