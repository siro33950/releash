use super::*;

#[test]
fn test_待ち時間_共通式で増加し上限後も値を返す() {
    // Given
    for policy in [
        RetryBackoff::ITEM,
        RetryBackoff::RECOVERY,
        RetryBackoff::CONFLICT,
        RetryBackoff::SERVICE,
    ] {
        // When / Then
        for count in [1, 2, 5, 100, u64::MAX] {
            let expected = (policy.initial.as_secs_f64()
                * policy
                    .multiplier
                    .powi(count.saturating_sub(1).min(i32::MAX as u64) as i32))
            .min(policy.maximum.as_secs_f64());
            for jitter in [0.8, 1.0, 1.2] {
                assert_eq!(
                    policy.delay(count, jitter),
                    Duration::from_secs_f64(expected * jitter)
                );
            }
        }
    }
}

#[test]
fn test_再試行頻度_初期100件の後は毎秒10件まで() {
    // Given
    let mut bucket = RetryBucket::new(Duration::ZERO);
    // When / Then
    for _ in 0..100 {
        assert_eq!(bucket.acquire(Duration::ZERO), Duration::ZERO);
    }
    assert_eq!(bucket.acquire(Duration::ZERO), Duration::from_millis(100));
    assert_eq!(
        bucket.acquire(Duration::from_millis(50)),
        Duration::from_millis(50)
    );
    assert_eq!(bucket.acquire(Duration::from_millis(100)), Duration::ZERO);
    assert_eq!(
        bucket.acquire(Duration::from_millis(100)),
        Duration::from_millis(100)
    );
    for _ in 0..100 {
        assert_eq!(bucket.acquire(Duration::from_secs(100)), Duration::ZERO);
    }
    assert!(!bucket.acquire(Duration::from_secs(100)).is_zero());
}

#[test]
fn test_ばらつき_uuidの固定bitに制限されず両側へ分散する() {
    let samples: Vec<_> = (0..1024).map(|_| jitter()).collect();
    assert!(samples.iter().all(|value| (0.8..=1.2).contains(value)));
    assert!(samples.iter().any(|value| *value < 0.9));
    assert!(samples.iter().any(|value| *value > 1.1));
}

#[tokio::test(start_paused = true)]
async fn test_やり直しの包み_判断に従って待ち時間を延ばし進み方を渡す() {
    // Given
    let limiter = RetryLimiter::deterministic();
    let started = tokio::time::Instant::now();
    let seen = std::sync::Mutex::new(Vec::new());
    // When
    let result = attempts(
        RetryBackoff::ITEM,
        &limiter,
        |error: &&str| match *error {
            "transient" => Some(AttemptProgress::Continue),
            "conflict" => Some(AttemptProgress::Reload),
            _ => None,
        },
        |progress| {
            let call = seen.lock().unwrap().len();
            seen.lock().unwrap().push((progress, started.elapsed()));
            async move {
                match call {
                    0 => Err("transient"),
                    1 => Err("conflict"),
                    2 => Err("transient"),
                    _ => Ok(42),
                }
            }
        },
    )
    .await;
    // Then
    assert_eq!(result, Ok(42));
    assert_eq!(
        *seen.lock().unwrap(),
        [
            (AttemptProgress::Continue, Duration::ZERO),
            (AttemptProgress::Continue, Duration::from_millis(5)),
            (AttemptProgress::Reload, Duration::from_millis(55)),
            (AttemptProgress::Continue, Duration::from_millis(75)),
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn test_やり直しの包み_やり直さない判断で最後の失敗を返す() {
    let limiter = RetryLimiter::deterministic();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let result = attempts(
        RetryBackoff::ITEM,
        &limiter,
        |_: &&str| None,
        |_| {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Err::<(), _>("corrupt") }
        },
    )
    .await;
    assert_eq!(result, Err("corrupt"));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn test_やり直しの頻度_多数の対象の再試行に共通の上限が掛かり待ち時間の後に消費する() {
    // Given
    let limiter = std::sync::Arc::new(RetryLimiter::deterministic());
    let times = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let started = tokio::time::Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..110 {
        let limiter = limiter.clone();
        let times = times.clone();
        tasks.push(tokio::spawn(async move {
            let calls = std::sync::atomic::AtomicUsize::new(0);
            attempts(
                RetryBackoff::ITEM,
                &limiter,
                |_: &&str| Some(AttemptProgress::Continue),
                |_| {
                    let first = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
                    let times = times.clone();
                    async move {
                        if first {
                            Err("busy")
                        } else {
                            times.lock().unwrap().push(started.elapsed());
                            Ok(())
                        }
                    }
                },
            )
            .await
            .unwrap();
        }));
    }
    // When
    for task in tasks {
        task.await.unwrap();
    }
    // Then
    let mut times = times.lock().unwrap().clone();
    times.sort();
    assert_eq!(times.len(), 110);
    assert!(times[0] >= Duration::from_millis(5));
    assert!(times[99] < Duration::from_millis(100));
    for (index, elapsed) in times.iter().enumerate().skip(100) {
        assert!(*elapsed >= Duration::from_millis((index as u64 - 99) * 100));
    }
}

#[tokio::test(start_paused = true)]
async fn test_期限の包み_期限で失敗を返し前に終われば結果を返す() {
    let started = tokio::time::Instant::now();
    let result = bounded(
        Duration::from_secs(20),
        || "expired",
        std::future::pending::<Result<(), &str>>(),
    )
    .await;
    assert_eq!(result, Err("expired"));
    assert_eq!(started.elapsed(), Duration::from_secs(20));
    assert_eq!(
        bounded(Duration::from_secs(20), || "expired", async {
            Ok::<_, &str>(1)
        })
        .await,
        Ok(1)
    );
}
