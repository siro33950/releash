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
