use super::*;

#[test]
fn test_作業列_重複を畳み実行中の追加を完了後に処理する() {
    // Given
    let mut queue = WorkQueue::default();
    queue.add("a", Duration::ZERO);
    queue.add("a", Duration::ZERO);
    queue.add("b", Duration::ZERO);
    // When / Then
    assert_eq!(queue.get(Duration::ZERO), Some("a"));
    queue.add("a", Duration::ZERO);
    assert_eq!(queue.get(Duration::ZERO), Some("b"));
    assert_eq!(queue.get(Duration::ZERO), None);
    assert_eq!(queue.failed(&"a"), 1);
    queue.done(&"a", Some(Duration::from_secs(1)), false);
    queue.add("a", Duration::ZERO);
    assert_eq!(queue.get(Duration::ZERO), None);
    assert_eq!(queue.get(Duration::from_secs(1)), Some("a"));
    assert_eq!(queue.failed(&"a"), 2);
    queue.done(&"a", None, true);
    assert_eq!(queue.get(Duration::from_secs(1)), None);
    queue.done(&"b", None, true);
    assert_eq!(queue.next_due(), None);
}

#[test]
fn test_再試行_分類から待ち時間を選び失敗回数を保持する() {
    // Given
    use crate::domain::{failure::FailureKind, retry::RetryBackoff};
    let mut queue = WorkQueue::default();
    queue.add("a", Duration::ZERO);
    queue.get(Duration::ZERO);
    // When / Then
    let due = queue.retry(
        &"a",
        FailureKind::RestartRequired,
        RetryBackoff::ITEM,
        Duration::ZERO,
        1.0,
    );
    assert_eq!(due, Duration::from_millis(10));
    queue.done(&"a", Some(due), false);
    assert_eq!(queue.get(Duration::from_millis(9)), None);
    assert_eq!(queue.get(due), Some("a"));
    let due = queue.retry(&"a", FailureKind::Temporary, RetryBackoff::ITEM, due, 1.0);
    assert_eq!(due, Duration::from_millis(20));
    assert_eq!(queue.failure_count(&"a"), 2);
    queue.done(&"a", None, true);
    assert_eq!(queue.failure_count(&"a"), 0);
}
