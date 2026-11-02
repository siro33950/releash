use releashd::test_support::integration::workflow::{
    WorkflowDirectoryFileService, WorkflowDirectoryService,
};

#[test]
fn test_定義フォルダ準備_存在しないフォルダを作って返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/workflows");
    let service = WorkflowDirectoryFileService::new(&path);
    // When
    let directory = service.prepare().unwrap();
    // Then
    assert!(path.is_dir());
    assert_eq!(directory.path(), path);
    assert_eq!(service.prepare().unwrap(), directory);
}

#[test]
fn test_定義フォルダ準備_作成失敗を返す() {
    // Given
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file");
    std::fs::write(&path, "blocks creation").unwrap();
    let service = WorkflowDirectoryFileService::new(path.join("workflows"));
    // When / Then
    assert!(service.prepare().is_err());
}
