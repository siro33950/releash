use super::*;
use futures_util::{FutureExt, StreamExt};

#[tokio::test(start_paused = true)]
async fn test_購読時計_一定間隔で通知し遅延した印は連続送信しない() {
    // Given
    let timer = TokioSubscriptionTimer;
    let interval = Duration::from_secs(10);
    let mut ticks = timer.interval(interval);
    assert!(ticks.next().now_or_never().is_none());
    ticks.next().await.unwrap();
    // When
    tokio::time::advance(interval * 3).await;
    ticks.next().await.unwrap();
    // Then
    assert!(ticks.next().now_or_never().is_none());
    tokio::time::advance(interval).await;
    assert_eq!(ticks.next().now_or_never(), Some(Some(())));
}

#[cfg(feature = "desktop")]
#[tokio::test(start_paused = true)]
async fn test_購読時計_指定時間まで待機する() {
    // Given
    let timer = TokioSubscriptionTimer;
    let mut wait = timer.sleep(Duration::from_secs(30));
    // When / Then
    assert!(wait.as_mut().now_or_never().is_none());
    tokio::time::advance(Duration::from_secs(29)).await;
    assert!(wait.as_mut().now_or_never().is_none());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(wait.now_or_never(), Some(()));
}
