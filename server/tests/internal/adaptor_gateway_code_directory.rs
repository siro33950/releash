use crate::test_support_git::{add_and_commit, create_initial_commit, create_test_repo};
use releashd::test_support::integration::code::{
    DirectoryQuery, DirectoryQueryGateway, WorktreeEntries,
};

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

#[test]
fn test_再帰列挙_除外項目も辿りgitとsymlinkの中へ入らず親から渡す() {
    // Given
    use releashd::test_support::integration::code::WalkStep;
    let (_dir, repo) = create_test_repo();
    let root = repo.workdir().unwrap();
    std::fs::create_dir_all(root.join("ignored/nested")).unwrap();
    std::fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    std::fs::write(root.join("ignored/nested/file"), "x").unwrap();
    std::os::unix::fs::symlink(root, root.join("loop")).unwrap();
    let mut paths = Vec::new();
    // When
    DirectoryQueryGateway
        .walk(root.to_str().unwrap(), &mut |entry| {
            paths.push((entry.path, entry.ignored));
            WalkStep::Continue
        })
        .unwrap();
    // Then
    assert!(!paths
        .iter()
        .any(|(path, _)| path == ".git" || path.starts_with(".git/") || path.starts_with("loop/")));
    let parent = paths
        .iter()
        .position(|(path, _)| path == "ignored")
        .unwrap();
    let child = paths
        .iter()
        .position(|(path, _)| path == "ignored/nested/file")
        .unwrap();
    assert!(parent < child);
    assert!(paths[child].1);
}

#[test]
fn test_再帰列挙_枝の省略と終了と読取失敗を返す() {
    // Given
    use releashd::test_support::integration::code::WalkStep;
    let (_dir, repo) = create_test_repo();
    let root = repo.workdir().unwrap();
    std::fs::create_dir(root.join("skip")).unwrap();
    std::fs::write(root.join("skip/file"), "x").unwrap();
    let mut paths = Vec::new();
    // When
    DirectoryQueryGateway
        .walk(root.to_str().unwrap(), &mut |entry| {
            paths.push(entry.path);
            WalkStep::SkipChildren
        })
        .unwrap();
    let mut count = 0;
    DirectoryQueryGateway
        .walk(root.to_str().unwrap(), &mut |_| {
            count += 1;
            WalkStep::Stop
        })
        .unwrap();
    // Then
    assert!(!paths.iter().any(|path| path == "skip/file"));
    assert_eq!(count, 1);
    assert!(DirectoryQueryGateway
        .walk(root.join("missing").to_str().unwrap(), &mut |_| {
            WalkStep::Continue
        })
        .is_err());
}

#[test]
fn test_再帰列挙_読めない子を飛ばし他の項目を続ける() {
    use releashd::test_support::integration::code::WalkStep;
    use std::os::unix::fs::PermissionsExt;
    // Given
    let (_dir, repo) = create_test_repo();
    let root = repo.workdir().unwrap();
    let unreadable = root.join("unreadable");
    std::fs::create_dir(&unreadable).unwrap();
    std::fs::write(unreadable.join("hidden"), "x").unwrap();
    std::fs::write(root.join("visible"), "x").unwrap();
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0)).unwrap();
    let mut paths = Vec::new();
    // When
    let result = DirectoryQueryGateway.walk(root.to_str().unwrap(), &mut |entry| {
        paths.push(entry.path);
        WalkStep::Continue
    });
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o700)).unwrap();
    // Then
    result.unwrap();
    assert!(paths.iter().any(|path| path == "visible"));
    assert!(!paths.iter().any(|path| path == "unreadable/hidden"));
}

#[test]
fn test_再帰列挙_取得後に消えた子を飛ばす() {
    use releashd::test_support::integration::code::WalkStep;
    // Given
    let (_dir, repo) = create_test_repo();
    let root = repo.workdir().unwrap();
    std::fs::create_dir(root.join("child")).unwrap();
    let mut paths = Vec::new();
    // When
    DirectoryQueryGateway
        .walk(root.to_str().unwrap(), &mut |entry| {
            if entry.directory {
                std::fs::remove_dir(root.join(&entry.path)).unwrap();
            }
            paths.push(entry.path);
            WalkStep::Continue
        })
        .unwrap();
    // Then
    assert_eq!(paths, ["child"]);
}

#[cfg(target_os = "linux")]
#[test]
fn test_再帰列挙_非utf8名を飛ばし直下一覧は失敗する() {
    use releashd::test_support::integration::code::WalkStep;
    use std::os::unix::ffi::OsStringExt;
    // Given
    let (_dir, repo) = create_test_repo();
    let root = repo.workdir().unwrap();
    std::fs::write(root.join(std::ffi::OsString::from_vec(vec![0xff])), "x").unwrap();
    std::fs::write(root.join("visible"), "x").unwrap();
    let mut paths = Vec::new();
    // When
    DirectoryQueryGateway
        .walk(root.to_str().unwrap(), &mut |entry| {
            paths.push(entry.path);
            WalkStep::Continue
        })
        .unwrap();
    // Then
    assert_eq!(paths, ["visible"]);
    assert!(DirectoryQueryGateway
        .entries(root.to_str().unwrap(), ".")
        .is_err());
}
