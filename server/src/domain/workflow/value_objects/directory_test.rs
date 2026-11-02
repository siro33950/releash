use super::*;

#[test]
fn test_定義フォルダ_保持したパスを外部編集に渡す() {
    // Given
    let path = PathBuf::from("/workflows");
    let directory = WorkflowDirectory::new(path.clone());
    // When / Then
    assert_eq!(directory.path(), path);
    assert_eq!(directory.editor_paths(), [path]);
}
