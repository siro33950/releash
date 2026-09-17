use super::*;
use crate::domain::daemon_supervision::DaemonExit;
use crate::usecase::test_helpers::{restore_desktop, tick, FakeDaemon};
use std::sync::atomic::Ordering;

#[tokio::test(start_paused = true)]
async fn test_起動監督_期限直前の接続は次の巡回が期限後でも停止せず復元できる() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(29_800).await;
    // When
    gateway.ready.store(true, Ordering::SeqCst);
    tick(100).await;
    // Then
    assert_eq!(supervisor.status().phase, "restoring");
    assert!(supervisor.connection().is_ok());
    // When
    tick(200).await;
    restore_desktop(&supervisor).await;
    // Then
    assert_eq!(supervisor.status().phase, "ready");
    assert_eq!(supervisor.status().stage, None);
    assert_eq!(supervisor.status().retries, 0);
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
}

#[tokio::test(start_paused = true)]
async fn test_起動監督_期限内の接続完了が期限後に届いても復元できる() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.connection_delay_ms.store(50, Ordering::SeqCst);
    gateway
        .connection_delivery_delay_ms
        .store(150, Ordering::SeqCst);
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(29_800).await;
    gateway.ready.store(true, Ordering::SeqCst);
    // When
    tick(100).await;
    tokio::time::advance(std::time::Duration::from_millis(50)).await;
    tokio::task::yield_now().await;
    tick(200).await;
    // Then
    assert_eq!(supervisor.status().phase, "restoring");
    assert_eq!(supervisor.connection().unwrap().connected_at_ms, 29_950);
    restore_desktop(&supervisor).await;
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
        let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
    restore_desktop(&supervisor).await;
    assert_eq!(supervisor.status().phase, "ready");
}

#[tokio::test(start_paused = true)]
async fn test_起動監督_初期化失敗の待機中にquitすると再spawnしない() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
        let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
        let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(200).await;
    restore_desktop(&supervisor).await;
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
async fn test_通常quit_判断待ちでも有限期限後に子を回収して終了する() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.ready.store(true, Ordering::SeqCst);
    *gateway.shutdown_response.lock() = Some(Ok(
        crate::domain::daemon_supervision::ShutdownResponse::DecisionRequired(
            "approval required".into(),
        ),
    ));
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(200).await;
    // When
    supervisor.stop(StopIntent::Quit(0)).unwrap();
    tick(1_000).await;
    // Then
    assert_eq!(supervisor.status().phase, "stopping");
    assert_eq!(
        supervisor.status().reason.as_deref(),
        Some("approval required")
    );
    // When
    tick(20_000).await;
    // Then
    assert_eq!(supervisor.status().phase, "stopped");
    assert_eq!(
        *gateway.calls.lock(),
        ["spawn", "shutdown", "terminate_and_wait"]
    );
}

#[tokio::test(start_paused = true)]
async fn test_起動失敗_停止未確認は一度だけ停止を試み終了操作を提示する() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    *gateway.termination_error.lock() = Some("exit unconfirmed".into());
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
async fn test_通常要求_接続検証と状態復元の完了前および切替中は送信しない() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(200).await;
    // When
    let admission =
        supervisor.admit_client_command(crate::domain::daemon_supervision::ClientOperation::Normal);
    let send = supervisor
        .send_frame("launch", Some(ClientOperation::Normal), vec![])
        .await;
    // Then
    assert!(admission.is_err());
    assert_eq!(
        send.unwrap_err().state,
        crate::domain::client_operation::transmission::TransmissionFailure::NotSent
    );
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
    // When
    gateway.ready.store(true, Ordering::SeqCst);
    tick(200).await;
    restore_desktop(&supervisor).await;
    supervisor
        .send_frame("launch", Some(ClientOperation::Normal), vec![])
        .await
        .unwrap();
    // Then
    assert_eq!(*gateway.calls.lock(), ["spawn", "forward"]);
    // When
    supervisor.stop(StopIntent::Update).unwrap();
    // Then
    assert!(supervisor
        .send_frame("launch", Some(ClientOperation::Normal), vec![])
        .await
        .is_err());
    assert_eq!(*gateway.calls.lock(), ["spawn", "forward"]);
    supervisor
        .send_frame("launch", Some(ClientOperation::Shutdown), vec![])
        .await
        .unwrap();
    assert_eq!(*gateway.calls.lock(), ["spawn", "forward", "forward"]);
}

#[tokio::test(start_paused = true)]
async fn test_通常quit_終了確認不能でも完了扱いにせず期限後にui終了を選ぶ() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.ready.store(true, Ordering::SeqCst);
    *gateway.termination_error.lock() = Some("exit unconfirmed".into());
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
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
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(200).await;
    restore_desktop(&supervisor).await;
    // When
    assert!(supervisor
        .send_frame("old", Some(ClientOperation::Normal), vec![])
        .await
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
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(200).await;
    restore_desktop(&supervisor).await;
    // When
    gateway.ready.store(false, Ordering::SeqCst);
    tick(200).await;
    // Then
    assert_eq!(supervisor.status().phase, "starting");
    assert!(supervisor
        .admit_client_command(ClientOperation::Normal)
        .is_err());
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
    // When
    gateway.ready.store(true, Ordering::SeqCst);
    tick(200).await;
    restore_desktop(&supervisor).await;
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

#[test]
fn test_送信失敗dto_未送信と結果不明の区別をクライアントへ渡す() {
    assert_eq!(
        serde_json::to_value(DesktopSendError::not_sent("before")).unwrap(),
        serde_json::json!({"state":"not_sent","message":"before"})
    );
    assert_eq!(
        serde_json::to_value(DesktopSendError::write_attempted("during")).unwrap(),
        serde_json::json!({"state":"unknown","message":"during"})
    );
}

#[tokio::test(start_paused = true)]
async fn test_状態復元_取得失敗後は同じdaemonで再取得の完了を待つ() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.ready.store(true, Ordering::SeqCst);
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(200).await;
    let first = supervisor.status().connection_generation;
    // When
    supervisor.fail_restoration(first, "Settings: read failed".into());
    tick(60_000).await;
    // Then
    assert_eq!(supervisor.status().phase, "failed");
    assert_eq!(supervisor.status().stage, Some("state_restoration"));
    assert_eq!(
        supervisor.status().reason.as_deref(),
        Some("Settings: read failed")
    );
    assert!(supervisor.status().retry_available);
    assert!(supervisor
        .admit_client_command(ClientOperation::Normal)
        .is_err());
    // When
    supervisor.retry().unwrap();
    tick(200).await;
    supervisor.fail_restoration(first, "stale failure".into());
    // Then
    assert_eq!(supervisor.status().phase, "restoring");
    assert_eq!(supervisor.status().connection_generation, first + 1);
    assert!(supervisor
        .admit_client_command(ClientOperation::RestoreState)
        .is_ok());
    assert!(supervisor
        .admit_client_command(ClientOperation::Normal)
        .is_err());
    assert!(supervisor
        .finish_restoration("launch", "desktop", first)
        .await
        .is_err());
    // When
    restore_desktop(&supervisor).await;
    // Then
    assert_eq!(supervisor.status().phase, "ready");
    assert!(supervisor.status().reason.is_none());
    assert!(supervisor
        .admit_client_command(ClientOperation::Normal)
        .is_ok());
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
}

#[tokio::test(start_paused = true)]
async fn test_状態復元_完了確認の失敗も表示しquitで一括停止する() {
    // Given
    let gateway = Arc::new(FakeDaemon::default());
    gateway.ready.store(true, Ordering::SeqCst);
    *gateway.restoration_error.lock() = Some("Required state has not been restored".into());
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(200).await;
    // When
    assert!(supervisor
        .finish_restoration(
            "launch",
            "desktop",
            supervisor.status().connection_generation
        )
        .await
        .is_err());
    // Then
    assert_eq!(supervisor.status().phase, "failed");
    assert_eq!(
        supervisor.status().reason,
        gateway.restoration_error.lock().clone()
    );
    // When
    supervisor.stop(StopIntent::Quit(0)).unwrap();
    assert!(supervisor.retry().is_err());
    tick(20_000).await;
    // Then
    assert_eq!(supervisor.status().phase, "stopped");
    assert_eq!(
        *gateway.calls.lock(),
        ["spawn", "shutdown", "terminate_and_wait"]
    );
}

#[tokio::test(start_paused = true)]
async fn test_状態復元_期限超過後に戻った古い完了は再試行を完了させない() {
    // Given
    let wait = Arc::new(tokio::sync::Notify::new());
    let gateway = Arc::new(FakeDaemon::default());
    *gateway.restoration_wait.lock() = Some(wait.clone());
    gateway.ready.store(true, Ordering::SeqCst);
    let supervisor = DaemonSupervisionUsecase::start(gateway.clone());
    tick(200).await;
    let _ = supervisor.attach("desktop".into());
    let completion = {
        let supervisor = supervisor.clone();
        let generation = supervisor.status().connection_generation;
        tokio::spawn(async move {
            supervisor
                .finish_restoration("launch", "desktop", generation)
                .await
        })
    };
    // When
    tick(30_100).await;
    // Then
    assert_eq!(supervisor.status().stage, Some("state_restoration"));
    assert!(supervisor.status().reason.unwrap().contains("30 seconds"));
    // When
    supervisor.retry().unwrap();
    tick(200).await;
    wait.notify_one();
    assert!(completion.await.unwrap().is_err());
    tick(200).await;
    // Then
    assert_eq!(supervisor.status().phase, "restoring");
    assert!(supervisor
        .admit_client_command(ClientOperation::Normal)
        .is_err());
    // When
    wait.notify_one();
    restore_desktop(&supervisor).await;
    // Then
    assert_eq!(supervisor.status().phase, "ready");
    assert_eq!(*gateway.calls.lock(), ["spawn"]);
}
