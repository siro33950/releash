use releashd::test_support::integration::code::CodeError;
use releashd::test_support::integration::code::StagingGateway;
use releashd::test_support::integration::platform::Deadline;
use releashd::test_support::integration::platform::OperationContext;
use releashd::test_support::integration::platform::OperationStopped;
use releashd::test_support::integration::repository::StagingRepository;
use std::time::Duration;
use std::time::Instant;

#[tokio::test]
async fn test_index更新_排他待機中は期限で停止し解除後に更新できる() {
    use std::sync::Arc;
    // Given
    let (dir, repo) = crate::test_support_git::create_test_repo();
    crate::test_support_git::create_initial_commit(&repo);
    std::fs::write(dir.path().join("new.txt"), "new").unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(repo.path().join("releash-index.lock"))
        .unwrap();
    fs2::FileExt::lock_exclusive(&lock).unwrap();
    for stage in [true, false] {
        let context = OperationContext::new(
            Some(Deadline::new(Instant::now() + Duration::from_millis(30))),
            Arc::new(tokio_util::sync::CancellationToken::new()),
        );
        // When
        let result = releashd::test_support::integration::platform::scope(context, async {
            if stage {
                StagingGateway.stage(dir.path().to_str().unwrap(), vec![])
            } else {
                StagingGateway.unstage(dir.path().to_str().unwrap(), vec![])
            }
        })
        .await;
        // Then
        assert!(
            matches!(result, Err(CodeError::Technical(error)) if error == OperationStopped::Expired.into())
        );
        assert!(repo
            .index()
            .unwrap()
            .get_path(std::path::Path::new("new.txt"), 0)
            .is_none());
    }
    drop(lock);
    StagingGateway
        .stage(dir.path().to_str().unwrap(), vec![])
        .unwrap();
    let mut index = repo.index().unwrap();
    index.read(true).unwrap();
    assert!(index.get_path(std::path::Path::new("new.txt"), 0).is_some());
    StagingGateway
        .unstage(dir.path().to_str().unwrap(), vec![])
        .unwrap();
    index.read(true).unwrap();
    assert!(index.get_path(std::path::Path::new("new.txt"), 0).is_none());
}

pub(crate) mod staging_gateway_tests {

    use git2::Repository;
    use releashd::test_support::integration::code::git_stage;

    use releashd::test_support::integration::code::git_unstage;

    use releashd::test_support::integration::repository::get_git_status;

    use crate::test_support_git::*;

    use std::fs;
    use std::path::Path;

    fn index_file_content(repo: &Repository, path: &str) -> String {
        let mut index = repo.index().unwrap();
        index.read(true).unwrap();
        let entry = index.get_path(Path::new(path), 0).unwrap();
        let blob = repo.find_blob(entry.id).unwrap();
        std::str::from_utf8(blob.content()).unwrap().to_string()
    }

    #[cfg(unix)]
    #[test]
    fn test_全ステージ_リンク先のないsymlinkを登録する() {
        // Given
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        std::os::unix::fs::symlink("missing", dir.path().join("link")).unwrap();
        // When
        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();
        // Then
        let mut index = repo.index().unwrap();
        index.read(true).unwrap();
        let entry = index.get_path(Path::new("link"), 0).unwrap();
        assert_eq!(entry.mode, 0o120000);
        assert_eq!(repo.find_blob(entry.id).unwrap().content(), b"missing");
    }

    #[cfg(unix)]
    #[test]
    fn test_全アンステージ_通常変更と型変更を両方解除する() {
        // Given
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "link", "original", "file");
        add_and_commit(&repo, "modified", "before", "file");
        add_and_commit(&repo, "deleted", "deleted", "file");
        add_and_commit(&repo, "renamed", "renamed", "file");
        fs::write(dir.path().join("modified"), "after").unwrap();
        fs::remove_file(dir.path().join("deleted")).unwrap();
        fs::rename(dir.path().join("renamed"), dir.path().join("destination")).unwrap();
        fs::remove_file(dir.path().join("link")).unwrap();
        std::os::unix::fs::symlink("missing", dir.path().join("link")).unwrap();
        fs::write(dir.path().join("new.txt"), "new").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();
        // When
        git_unstage(dir.path().to_str().unwrap(), vec![]).unwrap();
        // Then
        assert_eq!(index_file_content(&repo, "link"), "original");
        assert_eq!(index_file_content(&repo, "modified"), "before");
        assert_eq!(index_file_content(&repo, "deleted"), "deleted");
        assert_eq!(index_file_content(&repo, "renamed"), "renamed");
        let mut index = repo.index().unwrap();
        index.read(true).unwrap();
        assert_eq!(index.get_path(Path::new("link"), 0).unwrap().mode, 0o100644);
        assert!(index.get_path(Path::new("new.txt"), 0).is_none());
        assert!(index.get_path(Path::new("destination"), 0).is_none());
        assert_eq!(
            fs::read_to_string(dir.path().join("modified")).unwrap(),
            "after"
        );
        assert!(!dir.path().join("deleted").exists());
        assert!(dir.path().join("destination").exists());
        git_unstage(dir.path().to_str().unwrap(), vec![]).unwrap();
    }

    #[test]
    fn test_全アンステージ_変更のないskip_worktreeを保持する() {
        // Given
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "excluded.txt", "excluded", "file");
        let mut index = repo.index().unwrap();
        let mut entry = index.get_path(Path::new("excluded.txt"), 0).unwrap();
        entry.flags |= git2::IndexEntryFlag::EXTENDED.bits();
        entry.flags_extended |= git2::IndexEntryExtendedFlag::SKIP_WORKTREE.bits();
        index.add(&entry).unwrap();
        index.write().unwrap();
        let loaded = Repository::open(dir.path()).unwrap();
        assert_eq!(
            loaded
                .index()
                .unwrap()
                .get_path(Path::new("excluded.txt"), 0)
                .unwrap()
                .flags_extended,
            entry.flags_extended
        );
        fs::remove_file(dir.path().join("excluded.txt")).unwrap();
        fs::write(dir.path().join("new.txt"), "new").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();
        // When
        index.read(true).unwrap();
        assert_eq!(
            index
                .get_path(Path::new("excluded.txt"), 0)
                .unwrap()
                .flags_extended,
            entry.flags_extended,
            "stage must preserve excluded entry"
        );
        git_unstage(dir.path().to_str().unwrap(), vec![]).unwrap();
        index.read(true).unwrap();
        assert_eq!(
            index
                .get_path(Path::new("excluded.txt"), 0)
                .unwrap()
                .flags_extended,
            entry.flags_extended
        );
        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();
        // Then
        index.read(true).unwrap();
        let retained = index.get_path(Path::new("excluded.txt"), 0).unwrap();
        assert_eq!(retained.flags_extended, entry.flags_extended);
        assert_eq!(retained.id, entry.id);
        assert!(index.get_path(Path::new("new.txt"), 0).is_some());
        assert!(!repo
            .status_file(Path::new("excluded.txt"))
            .unwrap()
            .is_index_deleted());
    }

    #[test]
    fn test_全アンステージ_競合エントリをheadの有無に従って戻す() {
        // Given
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "tracked", "original", "file");
        let mut index = repo.index().unwrap();
        let mut entry = index.get_path(Path::new("tracked"), 0).unwrap();
        index.remove_path(Path::new("tracked")).unwrap();
        entry.flags = (entry.flags & !0x3000) | 0x2000;
        entry.id = repo.blob(b"conflicted").unwrap();
        index.add(&entry).unwrap();
        entry.path = b"new-conflict".to_vec();
        index.add(&entry).unwrap();
        index.write().unwrap();
        assert!(index.has_conflicts());
        // When
        git_unstage(dir.path().to_str().unwrap(), vec![]).unwrap();
        // Then
        index.read(true).unwrap();
        assert!(!index.has_conflicts());
        assert_eq!(index_file_content(&repo, "tracked"), "original");
        assert!(index.get_path(Path::new("new-conflict"), 0).is_none());
    }

    #[tokio::test]
    pub async fn test_stage_特定ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("new.txt"), "hello").unwrap();

        git_stage(dir.path().to_str().unwrap(), vec!["new.txt".to_string()]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].index_status, "new");
        assert_eq!(statuses[0].worktree_status, "none");
    }

    #[tokio::test]
    pub async fn test_stage_全ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();

        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 2);
        for s in &statuses {
            assert_eq!(s.index_status, "new");
            assert_eq!(s.worktree_status, "none");
        }
    }

    #[tokio::test]
    pub async fn test_stage_削除ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");
        fs::remove_file(dir.path().join("file.txt")).unwrap();

        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].index_status, "deleted");
        assert_eq!(statuses[0].worktree_status, "none");
    }

    #[tokio::test]
    pub async fn test_stage_未追跡ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("untracked.txt"), "data").unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(before[0].worktree_status, "new");

        git_stage(
            dir.path().to_str().unwrap(),
            vec!["untracked.txt".to_string()],
        )
        .unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(after[0].index_status, "new");
        assert_eq!(after[0].worktree_status, "none");
    }

    #[tokio::test]
    pub async fn test_unstage_特定ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        git_unstage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].worktree_status, "new");
        assert_eq!(statuses[0].index_status, "none");
    }

    #[tokio::test]
    pub async fn test_unstage_全ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::write(dir.path().join("b.txt"), "b").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec![]).unwrap();

        git_unstage(dir.path().to_str().unwrap(), vec![]).unwrap();

        let statuses = get_git_status(dir.path().to_str().unwrap()).unwrap();
        for s in &statuses {
            assert_eq!(s.index_status, "none");
            assert_eq!(s.worktree_status, "new");
        }
    }

    #[tokio::test]
    pub async fn test_unstage_unborn_branch() {
        let (dir, _repo) = create_test_repo();
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        git_stage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let before = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(before[0].index_status, "new");

        git_unstage(dir.path().to_str().unwrap(), vec!["file.txt".to_string()]).unwrap();

        let after = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(after[0].index_status, "none");
        assert_eq!(after[0].worktree_status, "new");
    }
}
