use super::*;
use futures_util::{FutureExt, StreamExt};

#[tokio::test(start_paused = true)]
async fn test_購読時計_一定間隔で通知し遅延した印は連続送信しない() {
    // Given
    let interval = Duration::from_secs(10);
    let mut ticks = Box::pin(ticks_after(interval, interval));
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

#[tokio::test(start_paused = true)]
async fn test_遅延通知_指定期限までは通知せず期限に一度だけ通知する() {
    // Given
    let delay = delays(Duration::from_secs(300));
    let mut elapsed = delay();
    assert!(elapsed.next().now_or_never().is_none());
    // When
    tokio::time::advance(Duration::from_secs(299)).await;
    assert!(elapsed.next().now_or_never().is_none());
    tokio::time::advance(Duration::from_secs(1)).await;
    // Then
    assert_eq!(elapsed.next().now_or_never(), Some(Some(())));
    assert_eq!(elapsed.next().now_or_never(), Some(None));
}
