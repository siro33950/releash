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
        supervision.connected(now);
        // When
        supervision.failed_after_exit(failure(FailureStage::UnexpectedExit), true, now + 1);
        // Then
        assert!(!supervision.restart_due(now + delay));
        assert!(supervision.restart_due(now + delay + 1));
    }
    supervision.connected(7_003);
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
    supervision.connected(1_000);
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
        assert!(supervision.connected(1));
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
    model.connected(STARTUP_TIMEOUT_MS);
    // Then
    assert_eq!(model.phase(), Phase::Starting);
}

#[test]
fn test_停止結果不明_切替は止めたまま終了操作を受け付ける() {
    // Given
    let mut model = DaemonSupervision::new(0);
    model.connected(1);
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
    model.connected(1);
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
    let normal = model.shell_command_admitted(ShellOperation::Normal, true);
    let recovery = model.shell_command_admitted(ShellOperation::Supervision, true);
    // Then
    assert!(!normal);
    assert!(recovery);
    assert_eq!(
        model.desktop_action(false, true, false),
        DesktopAction::Show
    );
    assert_eq!(model.desktop_action(true, true, false), DesktopAction::Wait);
    // When
    model.connected(1);
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
fn test_認証接続_接続時にreadyとなり再接続後も起動済みと分類する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    assert!(model.connected(1));
    assert_eq!(model.phase(), Phase::Ready);
    assert!(model.shell_command_admitted(ShellOperation::Normal, true));
    // When
    model.connection_lost(2);
    assert!(model.connected(3));
    assert_eq!(model.phase(), Phase::Ready);
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
fn test_認証接続_起動期限内だけreadyへ遷移し他のphaseでは受理しない() {
    for now in [
        STARTUP_TIMEOUT_MS - 1,
        STARTUP_TIMEOUT_MS,
        STARTUP_TIMEOUT_MS + 1,
    ] {
        // Given
        let mut model = DaemonSupervision::new(0);
        // When
        let connected = model.connected(now);
        // Then
        assert_eq!(connected, now < STARTUP_TIMEOUT_MS);
        assert_eq!(
            model.phase(),
            if connected {
                Phase::Ready
            } else {
                Phase::Starting
            }
        );
    }
    // Given
    let mut model = DaemonSupervision::new(0);
    assert!(model.connected(1));
    // When / Then
    assert!(!model.connected(2));
    assert_eq!(model.phase(), Phase::Ready);
    model.begin_stop(StopIntent::Quit(0));
    assert!(!model.connected(3));
    assert_eq!(model.phase(), Phase::Stopping);
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
        let ready = model.desktop_action(hidden, first_ready, failure_window);
        // Then
        assert_eq!(ready, DesktopAction::Ready { show_window: show });
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
        assert_eq!(
            model.desktop_action(true, true, false),
            DesktopAction::Ready { show_window: false }
        );
    }
}

#[test]
fn test_接続待ち_起動と切替の途中は待機し失敗確定後は終了する() {
    // Given
    let mut supervision = DaemonSupervision::new(0);
    // When / Then
    assert!(supervision.connection_pending());
    supervision.connected(1);
    assert!(supervision.connection_pending());
    supervision.spawn_failed("failed".into(), 2);
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
fn test_生存判定_連続失敗が閾値に達したときだけ切断とする() {
    // Given
    let mut liveness = DaemonLiveness::default();
    // When / Then
    assert!(!liveness.failed());
    assert!(liveness.failed());
}

#[test]
fn test_生存判定_成功で連続失敗が消える() {
    // Given
    let mut liveness = DaemonLiveness::default();
    liveness.succeeded();
    liveness.failed();
    // When
    liveness.succeeded();
    // Then
    assert!(!liveness.failed());
}

#[test]
fn test_起動期限超過_最後の接続失敗の分類を保持する() {
    // Given
    let mut model = DaemonSupervision::new(0);
    let failure = Failure {
        stage: FailureStage::Connection(releashd::desktop_api::TechnicalFailureNature::TimedOut),
        reason: "State stream was silent".into(),
    };
    // When
    let interruption = model
        .startup_interruption(STARTUP_TIMEOUT_MS, Some(failure.clone()))
        .unwrap();
    model.startup_terminated(interruption, STARTUP_TIMEOUT_MS);
    // Then
    assert_eq!(model.failure(), Some(&failure));
    assert_eq!(model.phase(), Phase::Backoff);
}

#[test]
fn test_接続監督_分類付きの失敗を保持し再接続で解消する() {
    // Given
    let mut supervision = DaemonSupervision::new(0);
    let failure = Failure {
        stage: FailureStage::Connection(releashd::desktop_api::TechnicalFailureNature::TimedOut),
        reason: "State stream was silent".into(),
    };
    // When
    supervision.connection_failed(failure.clone());
    // Then
    assert_eq!(supervision.failure(), Some(&failure));
    // When
    assert!(supervision.connected(1));
    // Then
    assert_eq!(supervision.failure(), None);
}
