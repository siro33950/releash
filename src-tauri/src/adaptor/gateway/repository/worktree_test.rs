use super::*;

#[cfg(unix)]
#[test]
fn test_worktree識別パス_実体消失後も同じ表記を返す() {
    // Given
    let parent = tempfile::tempdir().unwrap();
    let alias = parent.path().join("alias");
    std::os::unix::fs::symlink(parent.path(), &alias).unwrap();
    let worktree = alias.join("worktree");
    std::fs::create_dir(&worktree).unwrap();
    let identity = path_to_worktree_identity(&worktree).unwrap();

    // When
    std::fs::remove_dir(&worktree).unwrap();

    // Then
    assert_eq!(path_to_worktree_identity(&worktree).unwrap(), identity);
    assert_eq!(
        identity,
        parent
            .path()
            .canonicalize()
            .unwrap()
            .join("worktree")
            .to_string_lossy()
    );
}

#[test]
fn test_worktree識別パス_解決エラーを返す() {
    // Given
    let path = Path::new("/invalid\0path");
    // When / Then
    assert!(path_to_worktree_identity(path).is_err());
}
