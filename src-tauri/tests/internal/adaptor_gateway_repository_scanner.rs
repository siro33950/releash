pub(crate) mod tests {

    use releash_lib::test_support::integration::platform::changes_diff_tree_entries;
    use releash_lib::test_support::integration::platform::diff_tree_entries;
    use releash_lib::test_support::integration::platform::staged_diff_tree_entries;
    use releash_lib::test_support::integration::platform::FileDiffStatDto;
    use releash_lib::test_support::integration::platform::FileStatusDto;
    use releash_lib::test_support::integration::platform::RepositoryScanner;
    use releash_lib::test_support::integration::repository::DefaultRepositoryScanner;
    use std::sync::Arc;

    #[test]
    pub fn default_scanner_matches_existing_usecase_read_models_for_real_repo() {
        let (dir, repo) = crate::test_support_git::create_test_repo();
        crate::test_support_git::create_initial_commit(&repo);
        crate::test_support_git::add_and_commit(&repo, "staged.txt", "before\n", "add staged");
        crate::test_support_git::add_and_commit(&repo, "unstaged.txt", "before\n", "add unstaged");
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();

        std::fs::write(dir.path().join("staged.txt"), "before\nafter\n").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new("staged.txt")).unwrap();
            index.write().unwrap();
        }
        std::fs::write(dir.path().join("unstaged.txt"), "changed\n").unwrap();
        std::fs::write(dir.path().join("untracked.txt"), "new\n").unwrap();

        let repository =
            Arc::new(releash_lib::test_support::integration::platform::build_repository_usecase());
        let code = Arc::new(releash_lib::test_support::integration::platform::build_code_usecase());
        let scanner = DefaultRepositoryScanner::new(repository.clone(), code.clone());
        let repo_path = dir.path().to_str().unwrap();

        releash_lib::test_support::integration::repository::reset_status_walk_count_for_tests();
        let snapshot = scanner.scan(repo_path).unwrap();
        assert_eq!(
            releash_lib::test_support::integration::repository::status_walk_count_for_tests(),
            1
        );

        let expected_status: Vec<FileStatusDto> =
            releash_lib::test_support::integration::repository::get_git_status(repo_path)
                .unwrap()
                .into_iter()
                .map(Into::into)
                .collect();
        let expected_diff_stats: Vec<FileDiffStatDto> =
            releash_lib::test_support::integration::repository::get_status_diff_stats(repo_path)
                .unwrap()
                .into_iter()
                .map(Into::into)
                .collect();
        assert_eq!(snapshot.status, expected_status);
        assert_eq!(snapshot.diff_stats, expected_diff_stats);
        assert_eq!(snapshot.dirty_count, 3);
        assert_eq!(
            scanner.scan_worktrees(repo_path).unwrap(),
            repository.list_working_worktrees(repo_path).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&snapshot.diff_file_tree).unwrap(),
            serde_json::to_value(
                code.build_diff_file_tree(diff_tree_entries(
                    &expected_status,
                    &expected_diff_stats,
                ))
            )
            .unwrap()
        );
        assert_eq!(
            serde_json::to_value(&snapshot.staged_diff_file_tree).unwrap(),
            serde_json::to_value(code.build_diff_file_tree(staged_diff_tree_entries(
                &expected_status,
                &expected_diff_stats,
            )))
            .unwrap()
        );
        assert_eq!(
            serde_json::to_value(&snapshot.changes_diff_file_tree).unwrap(),
            serde_json::to_value(code.build_diff_file_tree(changes_diff_tree_entries(
                &expected_status,
                &expected_diff_stats,
            )))
            .unwrap()
        );
    }

    #[tokio::test]
    pub async fn default_scanner_prunes_stale_branch_bases_after_committed_cold_start_scan() {
        let (dir, repo) = crate::test_support_git::create_test_repo();
        crate::test_support_git::create_initial_commit(&repo);
        let branch_name = repo.head().unwrap().shorthand().unwrap().to_string();
        {
            let mut config = repo.config().unwrap();
            config
                .set_str(&format!("branch.{branch_name}.releash-base"), "main")
                .unwrap();
            config
                .set_str("branch.deleted.releash-base", "main")
                .unwrap();
        }

        let repository =
            Arc::new(releash_lib::test_support::integration::platform::build_repository_usecase());
        let code = Arc::new(releash_lib::test_support::integration::platform::build_code_usecase());
        let scanner = Arc::new(DefaultRepositoryScanner::new(repository, code));
        let state = releash_lib::test_support::integration::platform::WorktreeState::new(
            dir.path().to_str().unwrap().to_string(),
            true,
            scanner,
            releash_lib::test_support::integration::subscriptions::test_subscriptions(),
            Arc::new(
                releash_lib::test_support::integration::platform::TestRepositoryStateWorkerRuntime,
            ),
            releash_lib::test_support::integration::subscriptions::scan_driver(
                std::time::Duration::ZERO,
            ),
        );

        state.invalidate(
            releash_lib::test_support::integration::platform::InvalidateReason::change(),
        );
        let mut ready = false;
        for _ in 0..100 {
            let snapshot = state.snapshot_for_read();
            if snapshot.version >= 1 && !snapshot.flags.loading {
                ready = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(ready, "timed out waiting for initial repository scan");

        let config = repo.config().unwrap();
        assert_eq!(
            config
                .get_string(&format!("branch.{branch_name}.releash-base"))
                .unwrap(),
            "main"
        );
        assert!(config.get_string("branch.deleted.releash-base").is_err());
    }
}
