use super::*;
use crate::domain::daemon_supervision::{DaemonExit, StopIntent};
use crate::usecase::test_helpers::{tick, FakeDaemon};
use std::sync::atomic::Ordering;

#[derive(Default)]
struct FakeUpdate {
    calls: parking_lot::Mutex<Vec<&'static str>>,
    install_fails: bool,
    install_wait: Option<Arc<tokio::sync::Notify>>,
}
#[async_trait::async_trait]
impl crate::usecase::desktop_update::DesktopUpdateGateway for FakeUpdate {
    async fn check(&self) -> Result<Option<crate::usecase::desktop_update::UpdateInfo>, String> {
        Ok(None)
    }
}
#[async_trait::async_trait]
impl crate::domain::daemon_supervision::DesktopUpdateInstaller for FakeUpdate {
    async fn download(&self) -> Result<(), String> {
        self.calls.lock().push("download");
        Ok(())
    }
    async fn install(&self) -> Result<(), String> {
        self.calls.lock().push("install");
        if let Some(wait) = &self.install_wait {
            wait.notified().await;
        }
        if self.install_fails {
            Err("install failed".into())
        } else {
            Ok(())
        }
    }
    fn restart(&self) -> Result<(), String> {
        self.calls.lock().push("restart");
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn test_更新_停止完了と終了の確認後だけ適用し失敗時は再起動しない() {
    for (completed, install_fails) in [(false, false), (true, true), (true, false)] {
        // Given
        let daemon = Arc::new(FakeDaemon::default());
        daemon.ready.store(true, Ordering::SeqCst);
        let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
        tick(200).await;
        let update = Arc::new(FakeUpdate {
            install_fails,
            ..Default::default()
        });
        let service = crate::usecase::desktop_update::DesktopUpdateUsecase::new(
            update.clone(),
            supervisor.clone(),
        );
        // When
        let task = tokio::spawn(async move { service.apply().await });
        tick(200).await;
        // Then
        assert_eq!(*update.calls.lock(), ["download"]);
        assert_eq!(supervisor.status().phase, "stopping");
        assert_eq!(daemon.calls.lock().last(), Some(&"shutdown"));
        // When
        *daemon.exit.lock() = Some(DaemonExit {
            success: true,
            shutdown_complete: completed,
            reason: "exit".into(),
        });
        tick(200).await;
        // Then
        assert_eq!(task.await.unwrap().is_ok(), completed && !install_fails);
        let expected = if !completed {
            vec!["download"]
        } else if install_fails {
            vec!["download", "install"]
        } else {
            vec!["download", "install", "restart"]
        };
        assert_eq!(*update.calls.lock(), expected);
        if install_fails {
            assert_eq!(supervisor.status().stage, Some("update"));
        }
        assert_eq!(daemon.starts.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn test_更新中quit_適用の完了まで終了を待ち再起動を抑止する() {
    // Given
    let daemon = Arc::new(FakeDaemon::default());
    daemon.ready.store(true, Ordering::SeqCst);
    let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
    tick(200).await;
    let wait = Arc::new(tokio::sync::Notify::new());
    let update = Arc::new(FakeUpdate {
        install_wait: Some(wait.clone()),
        ..Default::default()
    });
    let service = crate::usecase::desktop_update::DesktopUpdateUsecase::new(
        update.clone(),
        supervisor.clone(),
    );
    let task = tokio::spawn(async move { service.apply().await });
    tick(200).await;
    *daemon.exit.lock() = Some(DaemonExit {
        success: true,
        shutdown_complete: true,
        reason: "exit".into(),
    });
    tick(200).await;
    assert_eq!(supervisor.status().phase, "installing");
    // When
    supervisor.stop(StopIntent::Quit(0)).unwrap();
    tick(200).await;
    assert_eq!(supervisor.status().phase, "installing");
    // Then
    assert_eq!(*update.calls.lock(), ["download", "install"]);
    // When
    wait.notify_one();
    tick(200).await;
    task.await.unwrap().unwrap();
    // Then
    assert_eq!(supervisor.status().phase, "stopped");
    assert_eq!(supervisor.status().stop_intent, Some("quit"));
    assert_eq!(*update.calls.lock(), ["download", "install"]);
}

#[tokio::test(start_paused = true)]
async fn test_更新排他_適用中の二つ目の要求は副作用なしで拒否する() {
    // Given
    let daemon = Arc::new(FakeDaemon::default());
    daemon.ready.store(true, Ordering::SeqCst);
    let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
    tick(200).await;
    let wait = Arc::new(tokio::sync::Notify::new());
    let update = Arc::new(FakeUpdate {
        install_wait: Some(wait.clone()),
        ..Default::default()
    });
    let service = Arc::new(DesktopUpdateUsecase::new(
        update.clone(),
        supervisor.clone(),
    ));
    let applying = {
        let service = service.clone();
        tokio::spawn(async move { service.apply().await })
    };
    tick(200).await;
    *daemon.exit.lock() = Some(DaemonExit {
        success: true,
        shutdown_complete: true,
        reason: "done".into(),
    });
    tick(200).await;
    // When
    let error = service.apply().await.unwrap_err();
    // Then
    assert_eq!(error.to_string(), "An update is already in progress.");
    assert_eq!(*update.calls.lock(), ["download", "install"]);
    assert_eq!(*daemon.calls.lock(), ["spawn", "shutdown"]);
    // When
    wait.notify_one();
    applying.await.unwrap().unwrap();
    // Then
    assert_eq!(*update.calls.lock(), ["download", "install", "restart"]);
}

#[tokio::test(start_paused = true)]
async fn test_更新停止_受付以外の応答と利用者判断を挟む完了を区別する() {
    for response in [
        Err("shutdown rejected".to_string()),
        Ok(
            crate::domain::daemon_supervision::ShutdownResponse::DecisionRequired(
                "approval required".into(),
            ),
        ),
    ] {
        // Given
        let daemon = Arc::new(FakeDaemon::default());
        daemon.ready.store(true, Ordering::SeqCst);
        *daemon.shutdown_response.lock() = Some(response.clone());
        let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
        tick(200).await;
        let update = Arc::new(FakeUpdate::default());
        let service = DesktopUpdateUsecase::new(update.clone(), supervisor.clone());
        // When
        let applying = tokio::spawn(async move { service.apply().await });
        tick(200).await;
        // Then
        assert_eq!(*update.calls.lock(), ["download"]);
        if response.is_err() {
            assert_eq!(
                applying.await.unwrap().unwrap_err().to_string(),
                "shutdown rejected"
            );
            assert_eq!(supervisor.status().phase, "failed");
        } else {
            assert!(!applying.is_finished());
            assert_eq!(
                supervisor.status().reason.as_deref(),
                Some("approval required")
            );
            assert!(supervisor
                .admit_client_command(crate::domain::daemon_supervision::ClientOperation::Shutdown)
                .is_ok());
            assert!(supervisor
                .admit_client_command(crate::domain::daemon_supervision::ClientOperation::Normal)
                .is_err());
            // When: the existing shutdown coordinator confirms the user's decision and exits.
            *daemon.exit.lock() = Some(DaemonExit {
                success: true,
                shutdown_complete: true,
                reason: "done".into(),
            });
            tick(200).await;
            applying.await.unwrap().unwrap();
            // Then
            assert_eq!(*update.calls.lock(), ["download", "install", "restart"]);
        }
        assert_eq!(daemon.starts.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn test_更新中終了_インストールが戻らなくても期限でquitし後続restartを抑止する() {
    // Given
    let daemon = Arc::new(FakeDaemon::default());
    daemon.ready.store(true, Ordering::SeqCst);
    let supervisor = DaemonSupervisionUsecase::start(daemon.clone());
    tick(200).await;
    let wait = Arc::new(tokio::sync::Notify::new());
    let update = Arc::new(FakeUpdate {
        install_wait: Some(wait.clone()),
        ..Default::default()
    });
    let service = DesktopUpdateUsecase::new(update.clone(), supervisor.clone());
    let task = tokio::spawn(async move { service.apply().await });
    tick(200).await;
    *daemon.exit.lock() = Some(DaemonExit {
        success: true,
        shutdown_complete: true,
        reason: "done".into(),
    });
    tick(200).await;
    assert_eq!(supervisor.status().phase, "installing");
    // When
    supervisor.stop(StopIntent::Quit(0)).unwrap();
    tick(crate::domain::daemon_supervision::QUIT_TIMEOUT_MS + 100).await;
    // Then
    assert_eq!(
        supervisor.desktop_action(false, true, false),
        crate::domain::daemon_supervision::DesktopAction::Exit(0)
    );
    assert_eq!(daemon.starts.load(Ordering::SeqCst), 1);
    assert_eq!(*update.calls.lock(), ["download", "install"]);
    wait.notify_one();
    task.await.unwrap().unwrap();
    assert_eq!(*update.calls.lock(), ["download", "install"]);
}
