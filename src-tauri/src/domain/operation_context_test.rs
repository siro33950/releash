use super::*;
struct Cancelled;
impl Cancellation for Cancelled {
    fn is_cancelled(&self) -> bool {
        true
    }
}

#[test]
fn test_期限_早い方を採り境界で期限切れになる() {
    let now = Instant::now();
    let early = Deadline::new(now + Duration::from_secs(1));
    let late = Deadline::new(now + Duration::from_secs(2));
    assert_eq!(early.minimum(late), early);
    assert_eq!(late.minimum(early), early);
    assert_eq!(early.remaining(now), Duration::from_secs(1));
    assert!(early.is_expired(now + Duration::from_secs(1)));
    assert_eq!(
        early.remaining(now + Duration::from_secs(2)),
        Duration::ZERO
    );
}

#[test]
fn test_停止理由_期限切れと取り消しを区別する() {
    let now = Instant::now();
    let context = OperationContext::new(None, Arc::new(Cancelled));
    assert_eq!(context.check(now), Err(OperationStopped::Cancelled));
    assert_eq!(
        context.with_deadline(Deadline::new(now)).check(now),
        Err(OperationStopped::Expired)
    );
    assert_eq!(OperationContext::default().check(now), Ok(()));
    assert_eq!(
        OperationStopped::Cancelled.failure_kind(),
        FailureKind::Cancelled
    );
    assert_eq!(
        OperationStopped::Expired.failure_kind(),
        FailureKind::Expired
    );
}
