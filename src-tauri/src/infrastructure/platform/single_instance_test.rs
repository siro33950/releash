use super::*;

#[test]
fn test_二重起動_既存uiをアクティブにして新規の所有権を渡さない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let owner = acquire(directory.path(), move || {
        let _ = sender.send(());
    })
    .unwrap()
    .unwrap();
    // When
    let secondary = acquire(directory.path(), || panic!("secondary listener")).unwrap();
    // Then
    assert!(secondary.is_none());
    receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    // When
    drop(owner);
    let replacement = acquire(directory.path(), || {}).unwrap();
    // Then
    assert!(replacement.is_some());
}
