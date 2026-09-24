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
