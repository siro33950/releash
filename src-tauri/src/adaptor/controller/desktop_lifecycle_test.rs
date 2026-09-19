use super::*;
use crate::domain::daemon_supervision::DaemonExit;
use crate::usecase::test_helpers::{restore_desktop, tick, FakeDaemon};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Default)]
struct Host {
    events: Arc<parking_lot::Mutex<Vec<String>>>,
    failure_window: Arc<AtomicBool>,
    restart_fails: bool,
}
impl DesktopHost for Host {
    fn has_failure_window(&self) -> bool {
        self.failure_window.load(Ordering::SeqCst)
    }
    fn ready(
        &self,
        _: crate::usecase::app_config::query_service::DesktopSettingsDto,
        first: bool,
        show: bool,
    ) {
        self.failure_window.store(false, Ordering::SeqCst);
        self.events.lock().push(format!("ready:{first}:{show}"));
    }
    fn show(&self) {
        self.failure_window.store(true, Ordering::SeqCst);
        self.events.lock().push("show".into());
    }
    fn exit(&self, code: i32) {
        self.events.lock().push(format!("exit:{code}"));
    }
    fn restart(&self) -> Result<(), String> {
        self.events.lock().push("restart".into());
        if self.restart_fails {
            Err("successor spawn failed".into())
        } else {
            self.exit(0);
            Ok(())
        }
    }
}

#[tokio::test(start_paused = true)]
async fn test_起動表示_ログイン引数と最小化設定の両方がある時だけ表示しない() {
    for (hidden, minimized, show) in [
        (true, true, false),
        (true, false, true),
        (false, true, true),
        (false, false, true),
    ] {
        // Given
        let daemon = Arc::new(FakeDaemon::default());
        daemon.start_minimized.store(minimized, Ordering::SeqCst);
        let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
        let host = Host::default();
        let task = tokio::spawn(observe_with(host.clone(), supervisor, hidden && minimized));
        tick(200).await;
        assert_eq!(
            *host.events.lock(),
            if show { vec!["show"] } else { vec![] }
        );
        host.events.lock().clear();
        // When
        daemon.ready.store(true, Ordering::SeqCst);
        tick(200).await;
        // Then
        assert_eq!(*host.events.lock(), [format!("ready:true:{show}")]);
        task.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn test_最小化起動_初回の異常終了と期限超過からの自動復旧でウィンドウを作らない() {
    for timeout in [false, true] {
        // Given
        let daemon = Arc::new(FakeDaemon::default());
        daemon.start_minimized.store(true, Ordering::SeqCst);
        let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
        let host = Host::default();
        let task = tokio::spawn(observe_with(host.clone(), supervisor.clone(), true));
        tick(200).await;
        // When
        if timeout {
            tick(30_000).await;
        } else {
            *daemon.exit.lock() = Some(DaemonExit {
                success: false,
                shutdown_complete: false,
                reason: "crashed during initialization".into(),
            });
            tick(200).await;
        }
        // Then
        assert_eq!(supervisor.status().phase, "backoff");
        assert_eq!(
            supervisor.status().stage,
            Some(if timeout {
                "startup_timeout"
            } else {
                "backend_initialization"
            })
        );
        assert!(supervisor.status().reason.is_some());
        assert!(host.events.lock().is_empty());
        assert!(!host.has_failure_window());
        // When
        tick(1_000).await;
        // Then
        assert_eq!(supervisor.status().phase, "starting");
        assert_eq!(daemon.starts.load(Ordering::SeqCst), 2);
        assert!(host.events.lock().is_empty());
        assert_eq!(
            *daemon.calls.lock(),
            if timeout {
                vec!["spawn", "terminate_and_wait", "spawn"]
            } else {
                vec!["spawn", "spawn"]
            }
        );
        // When
        daemon.ready.store(true, Ordering::SeqCst);
        tick(200).await;
        restore_desktop(&supervisor).await;
        tick(200).await;
        // Then
        assert_eq!(supervisor.status().phase, "ready");
        assert_eq!(
            *host.events.lock(),
            ["ready:true:false", "ready:false:false"]
        );
        assert!(!host.has_failure_window());
        task.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn test_最小化起動_自動再起動の対象外と上限到達では理由を表示して終了できる() {
    for (spawn_failure, success, failures, stage, reason) in [
        (true, false, 0, "spawn", "executable missing"),
        (false, true, 1, "backend_initialization", "exited"),
        (false, false, 4, "backend_initialization", "exited"),
    ] {
        // Given
        let daemon = Arc::new(FakeDaemon::default());
        daemon.spawn_failure.store(spawn_failure, Ordering::SeqCst);
        let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
        let host = Host::default();
        let task = tokio::spawn(observe_with(host.clone(), supervisor.clone(), true));
        tick(200).await;
        // When
        for attempt in 0..failures {
            *daemon.exit.lock() = Some(DaemonExit {
                success,
                shutdown_complete: false,
                reason: reason.into(),
            });
            tick(200).await;
            if attempt + 1 < failures {
                assert!(host.events.lock().is_empty());
                tick(4_000).await;
            }
        }
        // Then
        assert_eq!(supervisor.status().phase, "failed");
        assert_eq!(supervisor.status().stage, Some(stage));
        assert_eq!(supervisor.status().reason.as_deref(), Some(reason));
        assert!(supervisor.status().retry_available);
        assert_eq!(*host.events.lock(), ["show"]);
        assert!(host.has_failure_window());
        let starts = daemon.starts.load(Ordering::SeqCst);
        // When
        request_quit(&supervisor);
        tick(5_000).await;
        // Then
        assert_eq!(daemon.starts.load(Ordering::SeqCst), starts);
        assert_eq!(
            host.events.lock().last().map(String::as_str),
            Some("exit:0")
        );
        task.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn test_再接続表示_通常再接続では再表示せず失敗画面からの回復では表示する() {
    // Given
    let daemon = Arc::new(FakeDaemon::default());
    daemon.ready.store(true, Ordering::SeqCst);
    daemon.start_minimized.store(true, Ordering::SeqCst);
    let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
    let host = Host::default();
    let task = tokio::spawn(observe_with(host.clone(), supervisor, true));
    tick(200).await;
    for failure_window in [false, true] {
        // When
        *daemon.exit.lock() = Some(DaemonExit {
            success: false,
            shutdown_complete: false,
            reason: "crash".into(),
        });
        tick(200).await;
        host.failure_window.store(failure_window, Ordering::SeqCst);
        tick(5_000).await;
        // Then
        assert_eq!(
            host.events.lock().last(),
            Some(&format!("ready:false:{failure_window}"))
        );
    }
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn test_tray終了_停止要求から一括停止と子の終了確認後にだけuiを終了する() {
    // Given
    let daemon = Arc::new(FakeDaemon::default());
    daemon.ready.store(true, Ordering::SeqCst);
    let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
    let host = Host::default();
    let task = tokio::spawn(observe_with(host.clone(), supervisor.clone(), false));
    tick(200).await;
    // When
    crate::infrastructure::platform::tray::dispatch_menu_event(
        crate::infrastructure::platform::tray::ids::QUIT,
        || panic!("Quit must not dispatch Show"),
        || request_quit(&supervisor),
    );
    tick(5_000).await;
    // Then
    assert_eq!(*daemon.calls.lock(), ["spawn", "shutdown"]);
    assert!(!host
        .events
        .lock()
        .iter()
        .any(|event| event.starts_with("exit:")));
    *daemon.exit.lock() = Some(DaemonExit {
        success: true,
        shutdown_complete: true,
        reason: "done".into(),
    });
    tick(10_000).await;
    assert_eq!(
        host.events.lock().last().map(String::as_str),
        Some("exit:0")
    );
    assert_eq!(*daemon.calls.lock(), ["spawn", "shutdown"]);
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn test_native終了_終了を保留して一括停止と子の終了確認後にだけuiを終了する() {
    // Given: menu Quit, Cmd+Q, Dock Quit and AppleScript use applicationShouldTerminate:.
    let daemon = Arc::new(FakeDaemon::default());
    daemon.ready.store(true, Ordering::SeqCst);
    let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
    let host = Host::default();
    let task = tokio::spawn(observe_with(host.clone(), supervisor.clone(), false));
    tick(200).await;
    // When
    let reply =
        crate::infrastructure::platform::native_termination::dispatch_termination(true, || {
            request_quit(&supervisor);
        });
    tick(5_000).await;
    // Then: NSTerminateLater defers UI termination even after shutdown was accepted.
    assert_eq!(reply, 2);
    assert_eq!(*daemon.calls.lock(), ["spawn", "shutdown"]);
    assert!(!host
        .events
        .lock()
        .iter()
        .any(|event| event.starts_with("exit:")));
    // When
    *daemon.exit.lock() = Some(DaemonExit {
        success: true,
        shutdown_complete: true,
        reason: "done".into(),
    });
    tick(10_000).await;
    // Then
    assert_eq!(
        host.events.lock().last().map(String::as_str),
        Some("exit:0")
    );
    assert_eq!(*daemon.calls.lock(), ["spawn", "shutdown"]);
    assert_eq!(
        crate::infrastructure::platform::native_termination::dispatch_termination(false, || {
            panic!("completed shutdown must not be requested again");
        }),
        1
    );
    task.abort();
}

#[tokio::test(start_paused = true)]
async fn test_再起動_一括停止未確認と後継spawn失敗ではui終了へ進まない() {
    for (completed, fails) in [(false, false), (true, true), (true, false)] {
        // Given
        let daemon = Arc::new(FakeDaemon::default());
        daemon.ready.store(true, Ordering::SeqCst);
        let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
        let host = Host {
            restart_fails: fails,
            ..Default::default()
        };
        let task = tokio::spawn(observe_with(host.clone(), supervisor.clone(), false));
        tick(200).await;
        // When
        supervisor.stop(StopIntent::Restart).unwrap();
        tick(200).await;
        assert!(!host.events.lock().iter().any(|event| event == "restart"));
        *daemon.exit.lock() = Some(DaemonExit {
            success: true,
            shutdown_complete: completed,
            reason: "done".into(),
        });
        tick(500).await;
        // Then
        assert_eq!(
            host.events.lock().iter().any(|event| event == "restart"),
            completed
        );
        assert_eq!(
            host.events.lock().iter().any(|event| event == "exit:0"),
            completed && !fails
        );
        if fails {
            assert_eq!(supervisor.status().stage, Some("restart"));
            assert_eq!(
                supervisor.status().reason.as_deref(),
                Some("successor spawn failed")
            );
        }
        task.abort();
    }
}
