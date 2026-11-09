use releash_desktop::test_support::integration::single_instance::acquire;

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

#[test]
fn test_移行前後の二重起動_旧uiを優先し両保存先で排他する() {
    use releash_desktop::test_support::integration::single_instance::acquire_all;
    // Given
    let directory = tempfile::tempdir().unwrap();
    let paths = [directory.path().join("new"), directory.path().join("old")];
    let (sender, receiver) = std::sync::mpsc::channel();
    let owner = acquire(&paths[1], move || {
        let _ = sender.send(());
    })
    .unwrap()
    .unwrap();
    // When / Then
    assert!(acquire_all(&paths, || {}).unwrap().is_none());
    receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(acquire(&paths[0], || {}).unwrap().is_some());
    drop(owner);
    let (sender, receiver) = std::sync::mpsc::channel();
    let _owner = acquire_all(&paths, move || {
        let _ = sender.send(());
    })
    .unwrap()
    .unwrap();
    for path in &paths {
        assert!(acquire(path, || {}).unwrap().is_none());
        receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
    }
}

#[cfg(unix)]
#[test]
fn test_保存先の別名_同じ実体を一度だけロックし二重起動を拒否する() {
    use releash_desktop::test_support::integration::single_instance::acquire_all;
    // Given
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("new");
    let legacy = root.path().join("old");
    std::fs::create_dir(&current).unwrap();
    std::os::unix::fs::symlink(&current, &legacy).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    // When
    let owner = acquire_all(&[current.clone(), legacy.clone()], move || {
        let _ = sender.send(());
    })
    .unwrap()
    .unwrap();
    // Then
    for path in [&current, &legacy] {
        assert!(acquire(path, || {}).unwrap().is_none());
        receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
    }
    drop(owner);
    assert!(acquire_all(&[legacy, current], || {}).unwrap().is_some());
}
