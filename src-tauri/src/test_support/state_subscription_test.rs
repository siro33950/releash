use super::*;

#[test]
fn test_テスト駆動_runtime外でも受け側を保持し送信を受理する() {
    // Given
    let sender = driver::<u8>(|| panic!("runtime外では駆動を起こさない"));
    // When / Then
    assert!(sender.send(1).is_ok());
    assert!(!read_driver().is_closed());
    assert!(!pending_read_driver().is_closed());
    assert!(!terminal_driver().is_closed());
    assert!(!repository_driver().is_closed());
}

#[tokio::test]
async fn test_テスト駆動_runtime内では渡された駆動を起こす() {
    // Given
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    // When
    let sender = driver(|| sender);
    sender.send(1_u8).unwrap();
    // Then
    assert_eq!(receiver.recv().await, Some(1));
}
