use super::*;

#[test]
fn test_エディタ対象_worktreeだけをフォルダとして開く() {
    // Given / When
    let target = EditorTarget::worktree("/work tree").unwrap();
    // Then
    assert_eq!(target.editor_paths(), vec![PathBuf::from("/work tree")]);
    assert_eq!(target.label(), "フォルダ");
    assert!(matches!(
        EditorTarget::worktree(""),
        Err(EditorError::InvalidInput(_))
    ));
}

#[test]
fn test_エディタ対象_worktreeと相対ファイルを順に一度に渡す() {
    // Given / When / Then
    for file in [
        "src/main.rs",
        "a b.txt",
        ".gitignore",
        "src/.github/a",
        "./src/a",
    ] {
        let target = EditorTarget::file("/work tree", file).unwrap();
        assert_eq!(
            target.editor_paths(),
            vec![
                PathBuf::from("/work tree"),
                PathBuf::from("/work tree").join(file)
            ]
        );
        assert_eq!(target.label(), "ファイル");
    }
}

#[test]
fn test_エディタ対象_空worktreeと外部pathとgit内部を拒む() {
    // Given / When / Then
    assert!(matches!(
        EditorTarget::file("", "a"),
        Err(EditorError::InvalidInput(_))
    ));
    for file in [
        "../a",
        "src/../../a",
        "/outside/a",
        "/work/a",
        ".git",
        ".git/config",
        "src/.git/config",
    ] {
        assert!(
            matches!(
                EditorTarget::file("/work", file),
                Err(EditorError::InvalidInput(_))
            ),
            "{file}"
        );
    }
}
