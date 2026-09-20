use super::*;

#[tokio::test]
async fn test_終了処理_telemetryの最終送信を待ちpanicは失敗として返す() {
    struct Telemetry(std::sync::mpsc::Sender<()>);
    impl Drop for Telemetry {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
        }
    }
    struct FailedTelemetry;
    impl Drop for FailedTelemetry {
        fn drop(&mut self) {
            panic!("telemetry failed");
        }
    }
    // Given
    let (sent, received) = std::sync::mpsc::channel();
    // When
    shutdown_telemetry(Telemetry(sent)).await.unwrap();
    // Then
    received.try_recv().unwrap();
    assert!(shutdown_telemetry(FailedTelemetry)
        .await
        .unwrap_err()
        .to_string()
        .contains("telemetry failed"));
}

#[tokio::test(start_paused = true)]
async fn test_終了処理_telemetryが停止しても呼び出し側の期限を遮らない() {
    struct Telemetry {
        started: Option<tokio::sync::oneshot::Sender<()>>,
        release: std::sync::mpsc::Receiver<()>,
    }
    impl Drop for Telemetry {
        fn drop(&mut self) {
            self.started.take().unwrap().send(()).unwrap();
            self.release.recv().unwrap();
        }
    }
    // Given
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, receiver) = std::sync::mpsc::channel();
    let task = tokio::spawn(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            shutdown_telemetry(Telemetry {
                started: Some(started),
                release: receiver,
            }),
        )
        .await
    });
    ready.await.unwrap();
    // When
    tokio::time::advance(std::time::Duration::from_secs(15)).await;
    let result = task.await.unwrap();
    release.send(()).unwrap();
    // Then
    assert!(result.is_err());
}

#[test]
fn test_終了要求_後続要求を保持せず最初のコードだけを渡す() {
    // Given
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let port = DaemonProcessActionPort(sender);
    port.execute(ApplicationQuitIntent::Exit { code: 23 })
        .unwrap();
    // When / Then
    for code in 0..1000 {
        assert!(port
            .execute(ApplicationQuitIntent::Restart { code })
            .is_err());
        assert_eq!(receiver.len(), 1);
    }
    assert_eq!(receiver.try_recv().unwrap(), 23);
    receiver.close();
    for code in 0..1000 {
        assert!(port.execute(ApplicationQuitIntent::Exit { code }).is_err());
        assert_eq!(receiver.len(), 0);
    }
}
