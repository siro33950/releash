use super::*;

#[test]
fn test_ばらつき_uuidの固定bitに制限されず両側へ分散する() {
    let runtime = TokioWorkQueueRuntime::default();
    let samples: Vec<_> = (0..1024).map(|_| runtime.jitter()).collect();
    assert!(samples.iter().all(|value| (0.8..=1.2).contains(value)));
    assert!(samples.iter().any(|value| *value < 0.9));
    assert!(samples.iter().any(|value| *value > 1.1));
}

#[tokio::test(start_paused = true)]
async fn test_借用する試行_20秒でexpiredを返してfutureを解放する() {
    // Given
    let runtime = TokioWorkQueueRuntime::default();
    let started = tokio::time::Instant::now();
    let mut entered = false;
    // When
    let result = tokio::time::timeout(
        Duration::from_secs(21),
        runtime.attempt(Box::pin(async {
            entered = true;
            std::future::pending().await
        })),
    )
    .await
    .expect("borrowed attempt must end at its 20 second deadline");
    // Then
    assert!(entered);
    assert_eq!(result.unwrap_err().kind, FailureKind::Expired);
    assert_eq!(started.elapsed(), Duration::from_secs(20));
}
