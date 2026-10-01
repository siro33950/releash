use super::*;
use crate::domain::daemon_supervision::DaemonExit;
use crate::usecase::test_helpers::{tick, FakeDaemon};
use std::sync::atomic::{AtomicBool, Ordering};

struct PausingStatusOutput {
    pause_on: &'static str,
    paused: AtomicBool,
    entered: std::sync::mpsc::Sender<()>,
    release: parking_lot::Mutex<std::sync::mpsc::Receiver<()>>,
    sent: parking_lot::Mutex<Vec<DaemonStatus>>,
}

impl PausingStatusOutput {
    fn deliver(&self, operation: &str, status: DaemonStatus) {
        if operation == self.pause_on && !self.paused.swap(true, Ordering::SeqCst) {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        self.sent.lock().push(status);
    }
}

impl DaemonStatusOutput for PausingStatusOutput {
    fn start(&self, _: String, status: DaemonStatus) {
        self.deliver("start", status);
    }
    fn stop(&self, _: &str) {}
    fn publish(&self, status: DaemonStatus) {
        self.deliver("publish", status);
    }
}

fn pausing_output(
    pause_on: &'static str,
) -> (
    Arc<PausingStatusOutput>,
    std::sync::mpsc::Receiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    let (entered, waiting) = std::sync::mpsc::channel();
    let (resume, release) = std::sync::mpsc::channel();
    (
        Arc::new(PausingStatusOutput {
            pause_on,
            paused: AtomicBool::new(false),
            entered,
            release: parking_lot::Mutex::new(release),
            sent: parking_lot::Mutex::new(Vec::new()),
        }),
        waiting,
        resume,
    )
}

#[tokio::test]
async fn test_起動状態の購読_初期通知中の変化が最後に届く() {
    // Given
    let (output, entered, release) = pausing_output("start");
    let supervisor =
        DaemonSupervisionUsecase::start(Arc::new(FakeDaemon::default()), output.clone());
    let starting = {
        let supervisor = supervisor.clone();
        std::thread::spawn(move || supervisor.subscribe_status("screen".into()))
    };
    entered
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    supervisor
        .state
        .lock()
        .supervision
        .begin_stop(StopIntent::Quit(0));
    // When
    let (finished, completion) = std::sync::mpsc::channel();
    let publishing = {
        let supervisor = supervisor.clone();
        std::thread::spawn(move || {
            supervisor.publish();
            finished.send(()).unwrap();
        })
    };
    assert!(completion
        .recv_timeout(std::time::Duration::from_millis(100))
        .is_err());
    release.send(()).unwrap();
    starting.join().unwrap();
    publishing.join().unwrap();
    // Then
    assert_eq!(output.sent.lock().last().unwrap().phase, "stopping");
}

#[tokio::test]
async fn test_起動状態の購読_並行した変化は古い通知で終わらない() {
    // Given
    let (output, entered, release) = pausing_output("publish");
    let supervisor =
        DaemonSupervisionUsecase::start(Arc::new(FakeDaemon::default()), output.clone());
    supervisor.subscribe_status("screen".into());
    supervisor
        .state
        .lock()
        .supervision
        .begin_stop(StopIntent::Quit(0));
    let first = {
        let supervisor = supervisor.clone();
        std::thread::spawn(move || supervisor.publish())
    };
    entered
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    supervisor.state.lock().supervision.uncoordinated_stop();
    // When
    let (finished, completion) = std::sync::mpsc::channel();
    let second = {
        let supervisor = supervisor.clone();
        std::thread::spawn(move || {
            supervisor.publish();
            finished.send(()).unwrap();
        })
    };
    assert!(completion
        .recv_timeout(std::time::Duration::from_millis(100))
        .is_err());
    release.send(()).unwrap();
    first.join().unwrap();
    second.join().unwrap();
    // Then
    assert_eq!(output.sent.lock().last().unwrap().phase, "stopped");
}

#[tokio::test(start_paused = true)]
async fn test_起動状態の購読_初期状態と変化を送り停止後は送らない() {
    struct Output {
        active: parking_lot::Mutex<std::collections::HashSet<String>>,
        sent: parking_lot::Mutex<Vec<DaemonStatus>>,
    }
    impl DaemonStatusOutput for Output {
        fn start(&self, id: String, status: DaemonStatus) {
            self.active.lock().insert(id);
            self.sent.lock().push(status);
        }
        fn stop(&self, id: &str) {
            self.active.lock().remove(id);
        }
        fn publish(&self, status: DaemonStatus) {
            if !self.active.lock().is_empty() {
                self.sent.lock().push(status);
            }
        }
    }
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    let output = Arc::new(Output {
        active: parking_lot::Mutex::new(std::collections::HashSet::new()),
        sent: parking_lot::Mutex::new(Vec::new()),
    });
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone(), output.clone());
    // When
    supervisor.subscribe_status("screen".into());
    // Then
    assert_eq!(output.sent.lock().len(), 1);
    assert_eq!(output.sent.lock()[0].phase, "starting");
    // When
    gateway.ready.store(true, Ordering::SeqCst);
    tick(200).await;
    // Then
    assert_eq!(output.sent.lock().last().unwrap().phase, "ready");
    // When
    supervisor.stop_status_subscription("screen");
    let count = output.sent.lock().len();
    gateway.ready.store(false, Ordering::SeqCst);
    tick(200).await;
    // Then
    assert_eq!(output.sent.lock().len(), count);
}

#[test]
fn test_監督状態_接続失敗の分類を画面用のstageへ写す() {
    // Given
    let mut supervision = DaemonSupervision::new(0);
    supervision.connection_failed(crate::domain::daemon_supervision::Failure {
        stage: FailureStage::Connection(crate::domain::failure::TechnicalFailureNature::TimedOut),
        reason: "State stream was silent".into(),
    });
    // When
    let status = snapshot(&supervision);
    // Then
    assert_eq!(status.stage, Some("backend_connection_timed_out"));
    assert_eq!(status.reason.as_deref(), Some("State stream was silent"));
}

#[tokio::test(start_paused = true)]
async fn test_起動監督_期限直前の接続は次の巡回が期限後でも停止せずreadyになる() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    tick(29_800).await;
    // When
    gateway.ready.store(true, Ordering::SeqCst);
    tick(100).await;
    // Then
    assert_eq!(supervisor.status().phase, "ready");
    assert!(supervisor.connection().is_ok());
    // When
    tick(200).await;

    // Then
    assert_eq!(supervisor.status().phase, "ready");
    assert_eq!(supervisor.status().stage, None);
    assert_eq!(supervisor.status().retries, 0);
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
}

#[tokio::test(start_paused = true)]
async fn test_起動監督_期限内の接続完了が期限後に届いてもreadyになる() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.connection_delay_ms.store(50, Ordering::SeqCst);
    gateway
        .connection_delivery_delay_ms
        .store(150, Ordering::SeqCst);
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    tick(29_800).await;
    gateway.ready.store(true, Ordering::SeqCst);
    // When
    tick(100).await;
    tokio::time::advance(std::time::Duration::from_millis(50)).await;
    tokio::task::yield_now().await;
    tick(200).await;
    // Then
    assert_eq!(supervisor.status().phase, "ready");
    assert_eq!(supervisor.connection().unwrap().connected_at_ms, 29_950);

    assert_eq!(supervisor.status().phase, "ready");
    assert_eq!(supervisor.status().stage, None);
    assert_eq!(supervisor.status().retries, 0);
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
}

#[tokio::test(start_paused = true)]
async fn test_起動監督_期限前に接続を始めても期限以降の完了は停止する() {
    for delay in [100, 200] {
        // Given
        let gateway = Arc::new(FakeDaemon::default());
        gateway.connection_delay_ms.store(delay, Ordering::SeqCst);
        let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
        tick(29_800).await;
        gateway.ready.store(true, Ordering::SeqCst);
        // When
        tick(300).await;
        // Then
        assert_eq!(supervisor.status().phase, "backoff");
        assert_eq!(supervisor.status().stage, Some("startup_timeout"));
        assert!(supervisor.connection().is_err());
        assert_eq!(*gateway.calls.lock(), ["spawn", "terminate_and_wait"]);
        tick(1_100).await;
        assert_eq!(
            *gateway.calls.lock(),
            ["spawn", "terminate_and_wait", "spawn"]
        );
    }
}

#[tokio::test(start_paused = true)]
async fn test_起動監督_期限超過は子の終了確認を挟んで再試行する() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    // When
    tick(30_100).await;
    // Then
    assert_eq!(supervisor.status().stage, Some("startup_timeout"));
    assert_eq!(*gateway.calls.lock(), ["spawn", "terminate_and_wait"]);
    tick(1_100).await;
    assert_eq!(
        *gateway.calls.lock(),
        ["spawn", "terminate_and_wait", "spawn"]
    );
}

#[tokio::test(start_paused = true)]
async fn test_起動監督_spawn失敗を表示し明示的な再試行だけ受け付ける() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.spawn_failure.store(true, Ordering::SeqCst);
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    // When
    tick(60_000).await;
    // Then
    assert_eq!(gateway.starts.load(Ordering::SeqCst), 1);
    assert_eq!(
        supervisor.status().reason.as_deref(),
        Some("executable missing")
    );
    assert!(supervisor.connection().is_err());
    gateway.spawn_failure.store(false, Ordering::SeqCst);
    gateway.ready.store(true, Ordering::SeqCst);
    supervisor.retry().unwrap();
    supervisor.retry().unwrap();
    tick(200).await;

    assert_eq!(supervisor.status().phase, "ready");
}

#[tokio::test(start_paused = true)]
async fn test_起動監督_初期化失敗の待機中にquitすると再spawnしない() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    tick(100).await;
    // When
    *gateway.exit.lock() = Some(DaemonExit {
        success: false,
        shutdown_complete: false,
        reason: "Database initialization failed".into(),
    });
    tick(100).await;
    // Then
    assert_eq!(supervisor.status().stage, Some("backend_initialization"));
    // When
    supervisor.stop(StopIntent::Quit(0)).unwrap();
    tick(10_000).await;
    // Then
    assert_eq!(supervisor.status().phase, "stopped");
    assert_eq!(gateway.starts.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn test_切替_プロセス終了だけでは停止完了とみなさない() {
    for completed in [false, true] {
        // Given
        let gateway = Arc::new(FakeDaemon::default());
        gateway.ready.store(true, Ordering::SeqCst);
        let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
        tick(200).await;
        // When
        supervisor.stop(StopIntent::Update).unwrap();
        tick(200).await;
        assert_eq!(supervisor.status().phase, "stopping");
        // When
        *gateway.exit.lock() = Some(DaemonExit {
            success: true,
            shutdown_complete: completed,
            reason: "exit".into(),
        });
        tick(10_000).await;
        // Then
        assert_eq!(
            supervisor.status().phase,
            if completed { "stopped" } else { "failed" }
        );
        assert_eq!(gateway.starts.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn test_接続先検証_異なるインスタンスを拒否して子の終了を待つ() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.ready.store(true, Ordering::SeqCst);
    gateway.wrong_identity.store(true, Ordering::SeqCst);
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    tick(200).await;
    // When
    tick(200).await;
    // Then
    assert_eq!(supervisor.status().stage, Some("connection_identity"));
    assert_eq!(supervisor.status().phase, "failed");
    assert!(supervisor.connection().is_err());
    assert_eq!(*gateway.calls.lock(), ["spawn", "terminate_and_wait"]);
    // When
    supervisor.stop(StopIntent::Quit(0)).unwrap();
    tick(200).await;
    // Then
    assert_eq!(supervisor.status().phase, "stopped");
}

#[tokio::test(start_paused = true)]
async fn test_終了要求_起動予約よりquitを優先して子を作らない() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    // When
    supervisor.stop(StopIntent::Quit(0)).unwrap();
    tick(1_000).await;
    // Then
    assert_eq!(supervisor.status().phase, "stopped");
    assert_eq!(gateway.starts.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn test_起動期限_子の終了未確認なら失敗を表示し次のspawnを行わない() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    *gateway.termination_error.lock() =
        Some("Daemon exit could not be confirmed after termination.".into());
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    // When
    tick(60_000).await;
    // Then
    assert_eq!(supervisor.status().stage, Some("shutdown"));
    assert_eq!(
        supervisor.status().reason,
        gateway.termination_error.lock().clone()
    );
    assert_eq!(gateway.starts.load(Ordering::SeqCst), 1);
    assert!(supervisor.connection().is_err());
    let _ = supervisor.retry();
    // When
    tick(60_000).await;
    // Then
    assert_eq!(gateway.starts.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn test_未接続の停止_終了確認の成否を表示して再spawnしない() {
    for failure in [None, Some("exit unconfirmed".to_string())] {
        // Given
        let gateway = Arc::new(FakeDaemon::default());
        *gateway.termination_error.lock() = failure.clone();
        let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
        tick(200).await;
        // When
        supervisor.stop(StopIntent::Update).unwrap();
        tick(60_000).await;
        // Then
        assert_eq!(supervisor.status().phase, "failed");
        assert_eq!(supervisor.status().stage, Some("shutdown"));
        assert_eq!(
            supervisor.status().reason,
            Some(failure.unwrap_or_else(|| {
                "Daemon shutdown outcome is unknown; switching is blocked.".into()
            }))
        );
        assert_eq!(*gateway.calls.lock(), ["spawn", "terminate_and_wait"]);
        assert_eq!(gateway.starts.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn test_通常終了_停止要求のないready後の終了でも再spawnしない() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.ready.store(true, Ordering::SeqCst);
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    tick(200).await;

    // When
    *gateway.exit.lock() = Some(DaemonExit {
        success: true,
        shutdown_complete: false,
        reason: "normal exit".into(),
    });
    tick(120_000).await;
    // Then
    assert_eq!(supervisor.status().phase, "failed");
    assert_eq!(supervisor.status().stage, Some("unexpected_exit"));
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
}

#[tokio::test(start_paused = true)]
async fn test_起動失敗_停止未確認は一度だけ停止を試み終了操作を提示する() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    *gateway.termination_error.lock() = Some("exit unconfirmed".into());
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    // When
    tick(120_000).await;
    // Then
    assert_eq!(supervisor.status().phase, "failed");
    assert!(!supervisor.status().retry_available);
    assert_eq!(
        supervisor.desktop_action(false, true, false),
        crate::domain::daemon_supervision::DesktopAction::Show
    );
    assert_eq!(*gateway.calls.lock(), ["spawn", "terminate_and_wait"]);
}

#[tokio::test(start_paused = true)]
async fn test_通常quit_終了確認不能でも完了扱いにせず期限後にui終了を選ぶ() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.ready.store(true, Ordering::SeqCst);
    *gateway.termination_error.lock() = Some("exit unconfirmed".into());
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    tick(200).await;
    // When
    supervisor.stop(StopIntent::Quit(0)).unwrap();
    tick(20_000).await;
    // Then
    assert_eq!(supervisor.status().phase, "failed");
    assert_eq!(
        supervisor.status().reason.as_deref(),
        Some("exit unconfirmed")
    );
    assert_eq!(
        supervisor.desktop_action(false, true, false),
        crate::domain::daemon_supervision::DesktopAction::Exit(0)
    );
    assert_eq!(
        *gateway.calls.lock(),
        ["spawn", "shutdown", "terminate_and_wait"]
    );
}

#[tokio::test(start_paused = true)]
async fn test_旧renderer要求_古いlaunchを拒否して検証済みdaemonを止めない() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.ready.store(true, Ordering::SeqCst);
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    tick(200).await;

    // When
    assert!(supervisor
        .validate_connection("old", env!("CARGO_PKG_VERSION"))
        .is_err());
    tick(200).await;
    // Then
    assert_eq!(supervisor.status().phase, "ready");
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
}

#[tokio::test(start_paused = true)]
async fn test_ws切断_生存中の子へ再接続し期限超過時だけ終了確認して再spawnする() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.ready.store(true, Ordering::SeqCst);
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    tick(200).await;

    // When
    gateway.ready.store(false, Ordering::SeqCst);
    tick(200).await;
    // Then
    assert_eq!(supervisor.status().phase, "starting");
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
    // When
    gateway.ready.store(true, Ordering::SeqCst);
    tick(200).await;

    // Then
    assert_eq!(supervisor.status().phase, "ready");
    assert_eq!(gateway.starts.load(Ordering::SeqCst), 1);
    // When
    gateway.ready.store(false, Ordering::SeqCst);
    tick(30_300).await;
    // Then
    assert_eq!(*gateway.calls.lock(), ["spawn", "terminate_and_wait"]);
    assert_eq!(supervisor.status().stage, Some("startup_timeout"));
    tick(1_100).await;
    assert_eq!(
        *gateway.calls.lock(),
        ["spawn", "terminate_and_wait", "spawn"]
    );
}

#[tokio::test(start_paused = true)]
async fn test_接続情報要求_起動中と切替中は拒否せず接続確立を待つ() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    for reconnect in [false, true] {
        if reconnect {
            gateway.ready.store(false, Ordering::SeqCst);
            tick(200).await;
        }
        let requesting = supervisor.clone();
        let request = tokio::spawn(async move { requesting.attach().await });
        // When
        tick(200).await;
        // Then
        assert!(!request.is_finished());
        gateway.ready.store(true, Ordering::SeqCst);
        tick(200).await;
        let endpoint = request.await.unwrap().unwrap();
        assert_eq!(endpoint.launch_id, "launch");
        assert_eq!(endpoint.endpoint.token, "client-only");
    }
}

#[tokio::test(start_paused = true)]
async fn test_接続情報要求_同一性不一致のdaemonを返さず確定した起動失敗を返す() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.wrong_identity.store(true, Ordering::SeqCst);
    let supervisor = crate::usecase::test_helpers::start_supervision(gateway.clone());
    let requesting = supervisor.clone();
    let request = tokio::spawn(async move { requesting.attach().await });
    // When
    tick(200).await;
    // Then
    assert!(!request.is_finished());
    gateway.ready.store(true, Ordering::SeqCst);
    tick(200).await;
    let error = request.await.unwrap().err().unwrap();
    assert!(error.to_string().contains("different daemon instance"));
    assert_eq!(supervisor.status().stage, Some("connection_identity"));
    assert_eq!(supervisor.status().phase, "failed");
}
