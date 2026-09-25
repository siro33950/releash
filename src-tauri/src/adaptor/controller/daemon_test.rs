use super::*;
use adaptor::gateway::local_event_store::store::LocalEventStoreOpenError as E;
use usecase::application_startup::StartupFailureKind as K;

#[test]
fn test_performance_fixture_replaces_both_provider_executables_without_affecting_defaults() {
    assert_eq!(
        select_provider_agent_executables(
            "claude".to_string(),
            "codex".to_string(),
            Some("/tmp/provider-fixture".to_string()),
        ),
        (
            "/tmp/provider-fixture".to_string(),
            "/tmp/provider-fixture".to_string(),
        )
    );
    assert_eq!(
        select_provider_agent_executables("claude".to_string(), "codex".to_string(), None,),
        ("claude".to_string(), "codex".to_string())
    );
}

#[test]
fn b071_store_open_failures_map_to_the_closed_safe_startup_vocabulary() {
    for (error, expected) in [
        (E::WriterLockHeld, K::StoreInUse),
        (E::StorageUnavailable, K::StorageUnavailable),
        (E::UnsupportedRuntime, K::UnsupportedRuntime),
        (E::UnsupportedStoreVersion, K::UnsupportedStoreVersion),
        (E::InitializationStateInvalid, K::InitializationStateInvalid),
        (E::StoreValidationFailed, K::StoreValidationFailed),
        (E::SchemaEvolutionFailed, K::SchemaEvolutionFailed),
    ] {
        assert_eq!(classify_startup_failure(error), expected);
    }
}

#[tokio::test]
async fn test_daemon終了_受信先が閉じた場合のエラーを維持する() {
    // Given
    let (sender, exit) = tokio::sync::mpsc::channel(1);
    drop(sender);
    let shutdown = Arc::new(usecase::application_lifecycle::test_helpers::FakeShutdown::default());
    // When
    let error = Daemon {
        shutdown: shutdown.clone(),
        exit,
    }
    .wait()
    .await
    .unwrap_err();
    // Then
    assert_eq!(error, "daemon exit channel closed");
    assert!(shutdown.calls.lock().unwrap().is_empty());
}

#[test]
fn test_daemon終了_成功と各段階の失敗と停止で完了通知と指定codeを返す() {
    use usecase::application_lifecycle::test_helpers::STAGES;
    for scenario in std::iter::once("success".to_string())
        .chain(
            ["fail", "block"]
                .into_iter()
                .flat_map(|mode| STAGES.map(|stage| format!("{mode}:{stage}"))),
        )
        .chain(["stop", "drain", "flush"].map(|stage| format!("terminal:block:{stage}")))
    {
        // Given
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "adaptor::controller::daemon::daemon_tests::test_daemon終了_subprocess",
                "--ignored",
                "--nocapture",
            ])
            .env("RELEASH_SHUTDOWN_TEST_CASE", &scenario)
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
        if let Some(blocked) = scenario.strip_prefix("terminal:block:") {
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
            assert!(stdout.contains("shutdown-deadline:15"));
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
fn test_daemon終了_subprocess() {
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
    let mut shutdown = usecase::application_lifecycle::test_helpers::FakeShutdown {
        delay: std::time::Duration::from_secs(2),
        ..Default::default()
    };
    for stage in usecase::application_lifecycle::test_helpers::STAGES {
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
        if let Some(blocked) = scenario.strip_prefix("terminal:block:") {
            let blocked = match blocked {
                "stop" => "stop",
                "drain" => "drain",
                "flush" => "flush",
                _ => unreachable!(),
            };
            let fixture = adaptor::gateway::workflow::workflow_host::test_helpers::Fixture::new(0);
            let mut pty = usecase::terminal_surface::io_usecase::io_usecase_tests::FakePtyGateway::new();
            let owner = domain::terminal_surface::TerminalSurfaceOwner::session(
                domain::workspace_tree::WorkspaceIdentity::new("/repo"), "session",
            ).unwrap();
            pty.shutdown_surfaces.push(domain::terminal_surface::entities::TerminalSurface {
                session_key: owner.stable_key(),
                owner,
                worktree_path: Some("/repo".into()),
                label: None,
                runtime_generation: 1.into(),
                process_state: domain::terminal_surface::TerminalProcessState::Running,
                checkpoint: domain::terminal_surface::TerminalSurfaceCheckpoint::empty(80, 24),
                latest_sequence: 0,
                last_output_at: None,
            });
            let (started, ready) = tokio::sync::oneshot::channel();
            let (_release, receiver) = std::sync::mpsc::channel();
            *pty.shutdown_gate.lock() = Some((blocked, started, receiver));
            let terminal = Arc::new(usecase::terminal_surface::application::TerminalSurfaceApplication::new(std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
                Arc::new(pty),
                Arc::new(adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new()),
            ));
            let server = infrastructure::local_api::LocalApiServerBinding::bind(fixture._directory.path().into())
                .unwrap().start(axum::Router::new(), &tokio::runtime::Handle::current());
            let shutdown = adaptor::gateway::application_lifecycle::DaemonShutdownGateway {
                workflow: Arc::new(usecase::workflow::WorkflowRuntimeUsecase::new(Arc::new(
                    adaptor::gateway::workflow::WorkflowRuntimeCommandGateway::new_with_driver(
                        fixture.app.clone(), Arc::new(fixture.host.clone()),
                    ),
                ), Arc::new(crate::adaptor::gateway::workflow::ExecutionTreeArchiveFactRepository::from_backend(crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend::Live(fixture.app.store.clone().unwrap()))))),
                terminal,
                server,
                stop_observer: Arc::new(|| {}),
                telemetry: parking_lot::Mutex::new(None),
            };
            tokio::spawn(async move {
                ready.await.unwrap();
                assert!(sender.is_closed());
                for code in 0..1000 {
                    assert!(sender.try_send(code).is_err());
                }
                tokio::time::advance(std::time::Duration::from_secs(15)).await;
            });
            Daemon { shutdown: Arc::new(shutdown), exit }.wait().await
        } else {
            Daemon { shutdown: Arc::new(shutdown), exit }.wait().await
        }
    }).unwrap_err();
    panic!("daemon wait returned: {error}");
}

#[tokio::test]
async fn test_daemon起動のarchive移行結線_未終了対象をabortし事実保存後に旧ファイルを削除する() {
    use crate::adaptor::gateway::workflow::workflow_host::test_helpers::{
        archive_fixture, archive_workflow,
    };
    use crate::domain::workflow::{ExecutionStatus, ExecutionTreeArchiveRepository};
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
async fn test_開始計測_組み立て失敗前に起点を記録する() {
    use infrastructure::telemetry::metrics;
    // Given
    let _guard = metrics::lock_test_telemetry();
    metrics::reset_test_metrics();
    metrics::set_performance_configured(true);
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
    metrics::record_first_repo_snapshot_ready();
    metrics::record_first_repo_snapshot_ready();

    // Then
    assert!(result.is_err());
    let records = metrics::test_metric_records();
    let startup: Vec<_> = records
        .iter()
        .filter(|record| record.name == "releash.startup.duration_ms")
        .collect();
    assert_eq!(startup.len(), 1);
    assert!(startup[0].value >= 0.0);
    metrics::reset_test_metrics();
}
