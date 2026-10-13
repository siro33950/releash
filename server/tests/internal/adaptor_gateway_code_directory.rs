use crate::test_support_git::{add_and_commit, create_initial_commit, create_test_repo};
use releashd::test_support::integration::code::{DirectoryQuery, DirectoryQueryGateway};

#[test]
fn test_直下一覧_git状態と除外を含みgitを隠す() {
    // Given
    let (_dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let root = repo.workdir().unwrap();
    std::fs::create_dir(root.join("src")).unwrap();
    add_and_commit(&repo, "src/file.rs", "before\n", "file");
    add_and_commit(&repo, ".gitignore", "ignored\n", "ignore");
    std::fs::write(root.join("src/file.rs"), "after\n").unwrap();
    std::fs::write(root.join("ignored"), "hidden\n").unwrap();
    // When
    let statuses = [
        releashd::test_support::integration::platform::FileStatusDto {
            path: "src/file.rs".into(),
            index_status: "none".into(),
            worktree_status: "modified".into(),
        },
    ];
    let (app, _, _) = crate::adaptor_controller_client_workflow_mod::tests::make_read_only_app();
    let entries = app
        .client
        .app_state
        .as_ref()
        .unwrap()
        .code_usecase
        .directory_entries(root.to_str().unwrap(), ".", &statuses)
        .unwrap();
    // Then
    assert!(!entries.iter().any(|entry| entry.name == ".git"));
    let directory = entries.iter().find(|entry| entry.path == "src").unwrap();
    assert!(directory.directory);
    assert_eq!(
        directory.status,
        releashd::test_support::integration::code::GitStatus::Modified
    );
    assert!(
        entries
            .iter()
            .find(|entry| entry.path == "ignored")
            .unwrap()
            .ignored
    );
    assert!(!entries.iter().any(|entry| entry.path == "src/file.rs"));
    let children = DirectoryQueryGateway
        .entries(root.to_str().unwrap(), "src")
        .unwrap();
    assert_eq!(children[0].path, "src/file.rs");
}

#[test]
fn test_対象検証_親への移動と外向きsymlinkを拒否する() {
    // Given
    let (_dir, repo) = create_test_repo();
    let root = repo.workdir().unwrap();
    let outside = tempfile::TempDir::new().unwrap();
    std::fs::write(outside.path().join("file"), "private").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
    // When / Then
    for path in ["..", ".git", "link"] {
        assert!(DirectoryQueryGateway
            .entries(root.to_str().unwrap(), path)
            .is_err());
    }
    assert!(DirectoryQueryGateway
        .file_target(root.to_str().unwrap(), "link/file")
        .is_err());
    assert!(DirectoryQueryGateway
        .file_target(root.to_str().unwrap(), "deleted/file")
        .is_ok());
}
