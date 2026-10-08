use super::*;

fn limits() -> PriorityLimits {
    PriorityLimits::new(
        64,
        &[("interactive", 30), ("workflow", 40), ("default", 120)],
        50,
    )
}

#[test]
fn test_席数_全体64をsharesの比で切り上げて配る() {
    // Given
    let limits = limits();
    // When / Then
    assert_eq!(limits.available("interactive"), 11);
    assert_eq!(limits.available("workflow"), 14);
    assert_eq!(limits.available("default"), 41);
    assert_eq!(limits.queue_length("default"), 50);
    assert_eq!(
        PriorityLimits::new(1, &[("a", 1), ("b", 1000)], 1).available("a"),
        1
    );
}

#[tokio::test]
async fn test_受理_席が空いていれば待たずに通し解放で戻る() {
    // Given
    let limits = limits();
    // When
    let seat = limits.admit("interactive", None).await.unwrap();
    // Then
    assert_eq!(limits.available("interactive"), 10);
    drop(seat);
    assert_eq!(limits.available("interactive"), 11);
}

#[tokio::test]
async fn test_待ち行列_席が埋まると待ち空いたら通す() {
    // Given
    let limits = Arc::new(PriorityLimits::new(1, &[("only", 1)], 50));
    let held = limits.admit("only", None).await.unwrap();
    let waiting = {
        let limits = limits.clone();
        tokio::spawn(async move { limits.admit("only", None).await.map(|_| ()) })
    };
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());
    assert_eq!(limits.queue_length("only"), 49);
    // When
    drop(held);
    // Then
    assert_eq!(waiting.await.unwrap(), Ok(()));
    assert_eq!(limits.queue_length("only"), 50);
}

#[tokio::test]
async fn test_拒否_待ち行列が上限に達すると即拒否する() {
    // Given
    let limits = Arc::new(PriorityLimits::new(1, &[("only", 1)], 2));
    let _held = limits.admit("only", None).await.unwrap();
    let mut queued = Vec::new();
    for _ in 0..2 {
        let limits = limits.clone();
        queued.push(tokio::spawn(
            async move { limits.admit("only", None).await },
        ));
    }
    tokio::task::yield_now().await;
    // When
    let rejected = limits.admit("only", None).await.unwrap_err();
    // Then
    assert_eq!(
        rejected,
        Rejection {
            level: "only",
            reason: RejectReason::QueueFull
        }
    );
    assert_eq!(rejected.to_string(), "only requests rejected: queue_full");
    for task in queued {
        task.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn test_拒否_残り期限の4分の1を待っても空かなければ拒否する() {
    // Given
    let limits = PriorityLimits::new(1, &[("only", 1)], 50);
    let _held = limits.admit("only", None).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    // When
    let started = tokio::time::Instant::now();
    let rejected = limits.admit("only", Some(deadline)).await.unwrap_err();
    // Then
    assert_eq!(rejected.reason, RejectReason::TimedOut);
    assert_eq!(started.elapsed(), Duration::from_secs(30));
    assert_eq!(limits.queue_length("only"), 50);
}

#[tokio::test(start_paused = true)]
async fn test_拒否_期限が無ければ最大60秒待つ() {
    // Given
    let limits = PriorityLimits::new(1, &[("only", 1)], 50);
    let _held = limits.admit("only", None).await.unwrap();
    // When
    let started = tokio::time::Instant::now();
    let rejected = limits.admit("only", None).await.unwrap_err();
    // Then
    assert_eq!(rejected.reason, RejectReason::TimedOut);
    assert_eq!(started.elapsed(), Duration::from_secs(60));
}

#[tokio::test]
async fn test_優先度_段が違えば互いの枠に影響しない() {
    // Given
    let limits = PriorityLimits::new(2, &[("a", 1), ("b", 1)], 0);
    let _a = limits.admit("a", None).await.unwrap();
    assert_eq!(
        limits.admit("a", None).await.unwrap_err().reason,
        RejectReason::QueueFull
    );
    // When / Then
    assert!(limits.admit("b", None).await.is_ok());
}
