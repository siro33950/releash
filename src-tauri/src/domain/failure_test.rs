use super::*;

#[test]
fn test_失敗分類_再試行と要対応を全分類で決定する() {
    // Given / When / Then
    for kind in [
        FailureKind::Temporary,
        FailureKind::RestartRequired,
        FailureKind::StateRequired,
        FailureKind::InvalidInput,
        FailureKind::Expired,
        FailureKind::Missing,
        FailureKind::AlreadyPresent,
        FailureKind::Permission,
        FailureKind::Capacity,
        FailureKind::Unsupported,
        FailureKind::Internal,
        FailureKind::Corrupt,
        FailureKind::Cancelled,
        FailureKind::Unknown,
        FailureKind::OutsideRange,
        FailureKind::AuthenticationRequired,
    ] {
        let expected = match kind {
            FailureKind::Temporary => RetryAction::Retry,
            FailureKind::RestartRequired => RetryAction::Restart,
            _ => RetryAction::Stop,
        };
        assert_eq!(kind.retry_action(), expected);
        assert_eq!(
            kind.requires_attention(),
            expected == RetryAction::Stop && kind != FailureKind::Cancelled
        );
    }
}

#[test]
fn test_要対応_対象ごとの遷移だけを通知して成功で解除する() {
    let mut failures = TargetFailures::default();
    assert!(failures.observe("work", "a", FailureKind::StateRequired));
    assert!(!failures.observe("work", "a", FailureKind::StateRequired));
    assert!(!failures.observe("work", "b", FailureKind::Temporary));
    assert!(failures.requires_attention("work", "a"));
    assert!(!failures.requires_attention("work", "b"));
    assert!(failures.clear("work", "a"));
    assert!(!failures.requires_attention("work", "a"));
    assert!(!failures.clear("work", "a"));
}

#[test]
fn test_要対応_取消と再試行可能な失敗で以前の要対応を解除する() {
    // Given / When / Then
    for kind in [
        FailureKind::Cancelled,
        FailureKind::Temporary,
        FailureKind::RestartRequired,
    ] {
        let mut failures = TargetFailures::default();
        assert!(failures.observe("work", "a", FailureKind::StateRequired));
        assert!(failures.observe("work", "a", kind));
        assert!(!failures.requires_attention("work", "a"));
        assert!(!failures.observe("work", "a", kind));
    }
}
