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
