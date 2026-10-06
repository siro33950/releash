pub(crate) mod tests {

    use crate::test_support_git::*;
    use releash_lib::test_support::integration::platform::RepositoryUsecase;
    use releash_lib::test_support::integration::repository::resolve_file_watch_paths;
    use releash_lib::test_support::integration::repository::resolve_worktree_git_dir;
    use std::path::PathBuf;

    fn test_usecase() -> RepositoryUsecase {
        releash_lib::test_support::integration::platform::build_repository_usecase()
    }

    fn create_worktree(repo: &git2::Repository) -> (String, PathBuf, tempfile::TempDir) {
        static WT_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let id = WT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let wt_name = format!("wt-test-{}", id);
        let wt_dir = tempfile::TempDir::new().unwrap();
        let wt_path = wt_dir.path().join(&wt_name);
        repo.worktree(&wt_name, &wt_path, None).unwrap();
        (wt_name, wt_path, wt_dir)
    }

    #[test]
    pub fn test_git監視の対象_main_worktreeはrepositoryのgitディレクトリを返す() {
        // Given
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        // When
        let git_dir = resolve_worktree_git_dir(dir.path().to_str().unwrap()).unwrap();

        // Then
        assert_eq!(
            git_dir.canonicalize().unwrap(),
            dir.path().join(".git").canonicalize().unwrap()
        );
        assert!(git_dir.join("HEAD").exists());
        assert!(git_dir.join("refs/heads").exists());
    }

    #[test]
    pub fn test_git監視の対象_linked_worktreeは自分の登録ディレクトリを返す() {
        // Given
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");
        let (wt_name, wt_path, _wt_dir) = create_worktree(&repo);

        // When
        let git_dir = resolve_worktree_git_dir(wt_path.to_str().unwrap()).unwrap();

        // Then
        assert_eq!(
            git_dir.canonicalize().unwrap(),
            dir.path()
                .join(".git/worktrees")
                .join(wt_name)
                .canonicalize()
                .unwrap()
        );
        assert!(git_dir.join("index").exists());
    }

    #[test]
    pub fn test_git監視の対象_repositoryでないpathは失敗を返す() {
        assert!(resolve_worktree_git_dir("/nonexistent/path").is_err());
    }

    #[test]
    pub fn resolve_file_watch_paths_uses_only_current_worktree() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");
        let (_wt_name, wt_path, _wt_dir) = create_worktree(&repo);

        let paths = resolve_file_watch_paths(&test_usecase(), dir.path().to_str().unwrap());

        assert_eq!(paths, vec![dir.path().canonicalize().unwrap()]);
        assert!(!paths.contains(&wt_path.canonicalize().unwrap()));
    }
}
