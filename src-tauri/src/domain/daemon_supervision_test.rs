use super::*;

fn failure(stage: FailureStage) -> Failure {
    Failure {
        stage,
        reason: "test".into(),
    }
}

#[test]
fn test_起動監督_ready直後の連続クラッシュでも再起動は3回で止まる() {
    // Given
    let mut supervision = DaemonSupervision::new(0);
    // When
    for (now, delay) in [(0, 1_000), (1_001, 2_000), (3_002, 4_000)] {
        supervision.ready(now);
        // When
        supervision.failed_after_exit(failure(FailureStage::UnexpectedExit), true, now + 1);
        // Then
        assert!(!supervision.restart_due(now + delay));
        assert!(supervision.restart_due(now + delay + 1));
    }
    supervision.ready(7_003);
    supervision.failed_after_exit(failure(FailureStage::UnexpectedExit), true, 7_004);
    // Then
    assert_eq!(supervision.phase(), Phase::Failed);
    assert_eq!(supervision.retries(), 3);
    assert!(supervision.retry(8_000));
    assert_eq!(supervision.retries(), 0);
}

#[test]
fn test_起動監督_spawn失敗と正常終了は自動再起動しない() {
    for stage in [FailureStage::Spawn, FailureStage::UnexpectedExit] {
        // Given
        let mut supervision = DaemonSupervision::new(0);
        // When
        supervision.failed_after_exit(failure(stage), false, 1);
        // Then
        assert_eq!(supervision.phase(), Phase::Failed);
        assert!(!supervision.restart_due(u64::MAX));
    }
}

#[test]
fn test_起動監督_初期化失敗と起動期限超過は終了確認後に再起動する() {
    for stage in [FailureStage::Initialization, FailureStage::StartupTimeout] {
        // Given
        let mut supervision = DaemonSupervision::new(0);
        assert!(!supervision.startup_expired(29_999));
        assert!(supervision.startup_expired(30_000));
        // When
        supervision.failed_after_exit(failure(stage), true, 30_000);
        // Then
        assert_eq!(supervision.phase(), Phase::Backoff);
        assert_eq!(supervision.failure().unwrap().stage, stage);
    }
}

#[test]
fn test_起動監督_安定稼働後だけ試行回数をリセットする() {
    // Given
    let mut supervision = DaemonSupervision::new(0);
    // When
    supervision.failed_after_exit(failure(FailureStage::Initialization), true, 0);
    assert!(supervision.restart_due(1_000));
    supervision.ready(1_000);
    supervision.failed_after_exit(failure(FailureStage::UnexpectedExit), true, 61_000);
    // Then
    assert_eq!(supervision.retries(), 1);
}

#[test]
fn test_終了要求_待機中の予約を解除し停止不明では切替しない() {
    for intent in [StopIntent::Quit(0), StopIntent::Restart, StopIntent::Update] {
        // Given
        let mut supervision = DaemonSupervision::new(0);
        // When
        supervision.failed_after_exit(failure(FailureStage::Initialization), true, 0);
        assert!(supervision.begin_stop(intent));
        assert_eq!(supervision.stop_intent(), Some(intent));
        assert!(!supervision.restart_due(u64::MAX));
        supervision.stopped(false);
        // Then
        assert_eq!(supervision.phase(), Phase::Failed);
        assert!(!supervision.retry(0));
        // When
        supervision.stopped(true);
        // Then
        assert_eq!(supervision.phase(), Phase::Stopped);
    }
}

#[test]
fn test_接続検証_古いインスタンスと異なるリリースを拒否する() {
    // Given
    let cases = [
        ("old", env!("CARGO_PKG_VERSION")),
        ("new", "old"),
        ("new", env!("CARGO_PKG_VERSION")),
    ];
    // When
    let valid = cases.map(|(launch, release)| verify_identity("new", launch, release).is_ok());
    // Then
    assert_eq!(valid, [false, false, true]);
}

#[test]
fn test_終了要求_quit中は一括停止未完了の終了観測でもuiを終了し再起動しない() {
    for success in [false, true] {
        // Given
        let mut supervision = DaemonSupervision::new(0);
        assert!(supervision.ready(1));
        assert!(supervision.begin_stop(StopIntent::Quit(3)));
        assert_eq!(supervision.phase(), Phase::Stopping);
        // When
        supervision.observe_exit(
            DaemonExit {
                success,
                shutdown_complete: false,
                reason: "shutdown incomplete".into(),
            },
            2,
        );
        // Then
        assert_eq!(supervision.phase(), Phase::Stopped);
        assert_eq!(
            supervision.desktop_action(false, false, false),
            DesktopAction::Exit(3)
        );
        assert!(!supervision.restart_due(u64::MAX));
        assert!(!supervision.retry_available());
    }
}

#[test]
fn test_起動期限_期限後のreadyを拒否する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    // When
    model.ready(STARTUP_TIMEOUT_MS);
    // Then
    assert_eq!(model.phase(), Phase::Starting);
    assert!(!model.connection_admitted());
}

#[test]
fn test_停止結果不明_切替は止めたまま終了操作を受け付ける() {
    // Given
    let mut model = DaemonSupervision::new(0);
    model.ready(1);
    assert!(model.begin_stop(StopIntent::Update));
    // When
    model.stopped(false);
    // Then
    assert!(!model.begin_stop(StopIntent::Restart));
    assert!(model.begin_stop(StopIntent::Quit(0)));
    // When
    model.stopped(true);
    // Then
    assert_eq!(model.phase(), Phase::Stopped);
}

#[test]
fn test_接続拒否_終了確認までは通常画面と再試行を停止する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    model.ready(1);
    // When
    model.reject_connection(failure(FailureStage::Identity));
    // Then
    assert_eq!(model.phase(), Phase::Stopping);
    assert!(!model.retry_available());
    // When
    model.failed_after_exit(failure(FailureStage::Identity), false, 2);
    // Then
    assert!(model.retry_available());
}

#[test]
fn test_更新失敗_停止済みのまま失敗理由を表示する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    model.begin_stop(StopIntent::Update);
    model.stopped(true);
    // When
    model.stop_failed(FailureStage::Update, "install failed".into());
    // Then
    assert_eq!(model.phase(), Phase::Stopped);
    assert_eq!(model.failure().unwrap().reason, "install failed");
}

#[test]
fn test_更新適用_適用中のquitは完了後の行き先だけを変更する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    assert!(!model.begin_update_install());
    model.begin_stop(StopIntent::Update);
    model.stopped(true);
    assert!(model.begin_update_install());
    // When
    assert!(!model.begin_stop(StopIntent::Quit(0)));
    // Then
    assert_eq!(model.phase(), Phase::Installing);
    // When
    model.finish_update_install(None);
    // Then
    assert_eq!(model.phase(), Phase::Stopped);
    assert_eq!(model.stop_intent(), Some(StopIntent::Quit(0)));
}

#[test]
fn test_shellの受理判断_起動切替中は復旧と終了だけを許す() {
    // Given
    let mut model = DaemonSupervision::new(0);
    // When
    let normal = [ShellOperation::Normal, ShellOperation::ApplySettings]
        .map(|operation| model.shell_command_admitted(operation, true));
    let recovery = model.shell_command_admitted(ShellOperation::Supervision, true);
    // Then
    assert_eq!(normal, [false; 2]);
    assert!(recovery);
    assert_eq!(
        model.desktop_action(false, true, false),
        DesktopAction::Show
    );
    assert_eq!(model.desktop_action(true, true, false), DesktopAction::Wait);
    // When
    model.ready(1);
    // Then
    assert!(model.shell_command_admitted(ShellOperation::Normal, true));
    assert!(!model.shell_command_admitted(ShellOperation::Normal, false));
    // When
    model.begin_stop(StopIntent::Update);
    // Then
    assert!(!model.shell_command_admitted(ShellOperation::Normal, true));
}

#[test]
fn test_終了期限_終了未確認を完了とせずquitだけを許す() {
    // Given
    let mut model = DaemonSupervision::new(0);
    model.begin_stop(StopIntent::Quit(3));
    model.arm_quit_deadline(0);
    // When
    let expired = [QUIT_TIMEOUT_MS - 1, QUIT_TIMEOUT_MS].map(|now| model.quit_expired(now));
    // Then
    assert_eq!(expired, [false, true]);
    // When
    model.finish_quit_termination(Err("exit unconfirmed".into()));
    // Then
    assert_eq!(model.phase(), Phase::Failed);
    assert_eq!(model.failure().unwrap().reason, "exit unconfirmed");
    assert_eq!(
        model.desktop_action(false, true, false),
        DesktopAction::Exit(3)
    );
    assert!(model.update_stop_result().is_err());
    assert!(!model.retry_available());
}

#[test]
fn test_認証接続_表示の復元待ちも起動済みと分類し再接続を別世代にする() {
    // Given
    let mut model = DaemonSupervision::new(0);
    assert!(model.connected(1));
    assert_eq!(model.connection_generation(), 1);
    // When
    model.connection_lost(2);
    assert!(model.connected(3));
    assert_eq!(model.connection_generation(), 2);
    model.observe_exit(
        DaemonExit {
            success: false,
            shutdown_complete: false,
            reason: "crashed".into(),
        },
        4,
    );
    // Then
    assert_eq!(model.failure().unwrap().stage, FailureStage::UnexpectedExit);
    assert_eq!(model.phase(), Phase::Backoff);
}

#[test]
fn test_起動表示_起動待ちと初回接続と復旧の表示を一貫して決める() {
    for (hidden, first_ready, failure_window, show) in [
        (false, true, false, true),
        (true, true, false, false),
        (false, false, false, false),
        (true, false, false, false),
        (false, true, true, true),
        (true, true, true, false),
        (false, false, true, true),
        (true, false, true, true),
    ] {
        // Given
        let mut model = DaemonSupervision::new(0);
        assert_eq!(
            model.desktop_action(hidden, first_ready, failure_window),
            if hidden {
                DesktopAction::Wait
            } else {
                DesktopAction::Show
            }
        );
        // When
        assert!(model.connected(1));
        let restoring = model.desktop_action(hidden, first_ready, failure_window);
        assert!(model.ready(2));
        let ready = model.desktop_action(hidden, first_ready, failure_window);
        // Then
        assert_eq!(restoring, DesktopAction::Ready { show_window: show });
        assert_eq!(ready, restoring);
    }
}

#[test]
fn test_最小化起動_初回の自動再試行待ちと復旧後も表示しない() {
    for timeout in [false, true] {
        // Given
        let mut model = DaemonSupervision::new(0);
        // When
        if timeout {
            let interruption = model.startup_interruption(30_000, None).unwrap();
            model.startup_terminated(interruption, 30_000);
        } else {
            model.observe_exit(
                DaemonExit {
                    success: false,
                    shutdown_complete: false,
                    reason: "crashed during initialization".into(),
                },
                30_000,
            );
        }
        // Then
        assert_eq!(model.phase(), Phase::Backoff);
        assert_eq!(model.desktop_action(true, true, false), DesktopAction::Wait);
        assert_eq!(
            model.desktop_action(false, true, false),
            DesktopAction::Show
        );
        assert_eq!(
            model.desktop_action(true, false, false),
            DesktopAction::Show
        );
        // When
        assert!(model.restart_due(31_000));
        // Then
        assert_eq!(model.desktop_action(true, true, false), DesktopAction::Wait);
        // When
        assert!(model.connected(31_001));
        // Then
        for has_failure_window in [false, true] {
            assert_eq!(
                model.desktop_action(true, true, has_failure_window),
                DesktopAction::Ready { show_window: false }
            );
        }
        assert!(model.ready(31_002));
        assert_eq!(
            model.desktop_action(true, true, false),
            DesktopAction::Ready { show_window: false }
        );
    }
}

#[test]
fn test_状態復元_失敗を表示し再試行世代の完了後だけ受付を再開する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    model.connected(1);
    let first = model.connection_generation();
    // When
    assert!(model.fail_restoration(first, "Repositories: read failed".into()));
    // Then
    assert_eq!(model.phase(), Phase::Failed);
    assert_eq!(model.failure().unwrap().stage, FailureStage::Restoration);
    assert!(model.retry_available());
    assert!(model.connection_admitted());
    assert!(!model.restart_due(u64::MAX));
    // When
    assert!(model.retry_restoration(2));
    let current = model.connection_generation();
    model.begin_restoration("desktop".into(), 2);
    // Then
    assert_eq!(current, first + 1);
    assert!(!model.retry_restoration(3));
    assert!(!model.fail_restoration(first, "stale error".into()));
    assert!(!model.finish_restoration(first, "desktop", true, 3).is_ok());
    assert!(model.failure().is_none());
    assert!(model
        .finish_restoration(current, "desktop", true, 3)
        .is_ok());
    assert!(!model.fail_restoration(current, "late error".into()));
}

#[test]
fn test_状態復元_画面接続からの期限を超えたら失敗を示し遅い完了を拒否する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    model.connected(1);
    // When
    model.expire_restoration(60_000);
    // Then
    assert_eq!(model.phase(), Phase::Restoring);
    assert_eq!(
        model.desktop_action(true, true, false),
        DesktopAction::Ready { show_window: false }
    );
    // When
    model.begin_restoration("desktop".into(), 60_000);
    model.begin_restoration("desktop".into(), 70_000);
    model.expire_restoration(89_999);
    // Then
    assert_eq!(model.phase(), Phase::Restoring);
    assert!(!model
        .finish_restoration(model.connection_generation(), "desktop", true, 90_000)
        .is_ok());
    assert_eq!(model.phase(), Phase::Failed);
    assert!(model.failure().unwrap().reason.contains("30 seconds"));
    assert!(model.retry_restoration(90_001));
    model.expire_restoration(120_001);
    assert_eq!(model.phase(), Phase::Failed);
}

#[test]
fn test_状態復元_終了開始後の失敗と再試行と完了を拒否する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    model.connected(1);
    let generation = model.connection_generation();
    model.fail_restoration(generation, "read failed".into());
    // When
    model.begin_stop(StopIntent::Quit(0));
    // Then
    assert!(!model.fail_restoration(generation, "late error".into()));
    assert!(!model
        .finish_restoration(generation, "desktop", true, 2)
        .is_ok());
    assert!(!model.retry_restoration(2));
    assert!(!model.retry_available());
    assert_eq!(model.phase(), Phase::Stopping);
}

#[test]
fn test_接続待ち_起動と切替の途中は待機し失敗確定後は終了する() {
    // Given
    let mut supervision = DaemonSupervision::new(0);
    // When / Then
    assert!(supervision.connection_pending());
    supervision.connected(1);
    assert!(supervision.connection_pending());
    supervision.fail_restoration(supervision.connection_generation(), "failed".into());
    assert!(!supervision.connection_pending());
}

#[test]
fn test_接続待ち_stoppedは終了しbackoffとstoppingとinstallingは待機する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    // When / Then
    model.observe_exit(
        DaemonExit {
            success: false,
            shutdown_complete: false,
            reason: "exit".into(),
        },
        1,
    );
    assert_eq!(model.phase(), Phase::Backoff);
    assert!(model.connection_pending());
    model.begin_stop(StopIntent::Update);
    assert_eq!(model.phase(), Phase::Stopping);
    assert!(model.connection_pending());
    model.observe_exit(
        DaemonExit {
            success: true,
            shutdown_complete: true,
            reason: "stopped".into(),
        },
        2,
    );
    assert_eq!(model.phase(), Phase::Stopped);
    assert!(!model.connection_pending());
    assert!(model.begin_update_install());
    assert_eq!(model.phase(), Phase::Installing);
    assert!(model.connection_pending());
}

#[test]
fn test_復元完了_最新attachmentと接続と世代が揃ったときだけreadyになる() {
    // Given
    let mut model = DaemonSupervision::new(0);
    model.connected(1);
    let generation = model.connection_generation();
    model.begin_restoration("first".into(), 2);
    model.begin_restoration("second".into(), 3);
    // When / Then
    assert_eq!(
        model.finish_restoration(generation, "first", true, 4),
        Err("Desktop attachment changed during restoration".into())
    );
    assert_eq!(model.phase(), Phase::Failed);
    assert!(model.retry_restoration(5));
    let current = model.connection_generation();
    assert!(model
        .finish_restoration(generation, "second", true, 6)
        .is_err());
    assert_eq!(model.phase(), Phase::Restoring);
    assert_eq!(
        model.finish_restoration(current, "second", false, 6),
        Err("Desktop disconnected during restoration".into())
    );
    assert!(model.retry_restoration(7));
    model
        .finish_restoration(model.connection_generation(), "second", true, 8)
        .unwrap();
    assert_eq!(model.phase(), Phase::Ready);
    assert!(model
        .finish_restoration(model.connection_generation(), "second", true, 9)
        .is_err());
}
