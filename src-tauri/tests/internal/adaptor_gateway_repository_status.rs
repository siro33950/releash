use releashd::test_support::integration::repository::get_repository_status_scan;

use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support_git::create_initial_commit;
use crate::test_support_git::create_test_repo;

#[test]
pub fn test_差分行数_各操作の停止をゼロ行に変えず後続へ進まない() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    crate::test_support_git::add_and_commit(&repo, "file", "before\n", "file");
    std::fs::write(dir.path().join("file"), "after\nextra\n").unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| get_repository_status_scan(dir.path().to_str().unwrap()));
}
pub(crate) mod status_gateway_tests {
    use super::*;

    use crate::test_support_git::*;
    use releashd::test_support::integration::repository::get_git_status;
    use releashd::test_support::integration::repository::get_repository_status_scan;
    use releashd::test_support::integration::repository::get_status_diff_stats;
    use releashd::test_support::integration::repository::reset_status_walk_count_for_tests;
    use releashd::test_support::integration::repository::status_walk_count_for_tests;
    use std::fs;
    use std::path::Path;

    #[test]
    pub fn test_状態取得_未追跡ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        fs::write(dir.path().join("new_file.txt"), "hello").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "new_file.txt");
        assert_eq!(result[0].worktree_status, "new");
        assert_eq!(result[0].index_status, "none");
    }

    #[test]
    pub fn first_repo_snapshot_records_only_first_successful_status_scan() {
        let _guard = releashd::test_support::integration::telemetry::lock_test_telemetry();
        releashd::test_support::integration::telemetry::reset_test_metrics();
        releashd::test_support::integration::telemetry::set_performance_configured(true);
        releashd::test_support::integration::telemetry::set_performance_enabled(true);
        releashd::test_support::integration::telemetry::set_startup_origin(
            std::time::Instant::now() - std::time::Duration::from_millis(20),
        );

        let invalid = tempfile::TempDir::new().unwrap();
        assert!(get_repository_status_scan(invalid.path().to_str().unwrap()).is_err());
        assert!(
            !releashd::test_support::integration::telemetry::first_repo_snapshot_recorded_for_tests(
            )
        );
        assert!(
            releashd::test_support::integration::telemetry::test_metric_records()
                .iter()
                .all(|record| record.name != "releash.startup.duration_ms")
        );

        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        get_repository_status_scan(dir.path().to_str().unwrap()).unwrap();
        get_repository_status_scan(dir.path().to_str().unwrap()).unwrap();

        let startup_records: Vec<_> =
            releashd::test_support::integration::telemetry::test_metric_records()
                .into_iter()
                .filter(|record| record.name == "releash.startup.duration_ms")
                .collect();
        assert_eq!(startup_records.len(), 1);
        assert!(startup_records[0].value >= 20.0);
        assert!(startup_records[0].attributes.iter().any(|(key, value)| {
            key == "releash.operation" && value == "startup.first_repo_snapshot_ready"
        }));
        releashd::test_support::integration::telemetry::reset_test_metrics();
    }

    #[test]
    pub fn test_状態取得_ステージ済み() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join("staged.txt"), "content").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("staged.txt")).unwrap();
        index.write().unwrap();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "staged.txt");
        assert_eq!(result[0].index_status, "new");
    }

    #[test]
    pub fn test_状態取得_変更済み() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "original", "add file");

        fs::write(dir.path().join("file.txt"), "modified content").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "file.txt");
        assert_eq!(result[0].worktree_status, "modified");
    }

    #[test]
    pub fn test_状態取得_空リポジトリ() {
        let (dir, _repo) = create_test_repo();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    pub fn test_状態取得_無視ファイルはdefaultで除外する() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join(".gitignore"), "ignored.txt\nbuild/\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(".gitignore")).unwrap();
        index.write().unwrap();
        let sig = git2::Signature::now("Test User", "test@example.com").unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "add gitignore", &tree, &[&parent])
            .unwrap();

        fs::write(dir.path().join("ignored.txt"), "should be ignored").unwrap();
        fs::create_dir(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build").join("output.js"), "built").unwrap();

        let result = get_git_status(dir.path().to_str().unwrap()).unwrap();

        assert!(
            result.iter().all(|e| e.worktree_status != "ignored"),
            "ignored entries should not appear in default status"
        );
        assert!(result.iter().all(|e| e.path != "ignored.txt"));
        assert!(result.iter().all(|e| e.path != "build"));
    }

    #[test]
    pub fn repository_status_scan_matches_legacy_status_and_diff_stats_with_one_status_walk() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "staged.txt", "before\n", "add staged");
        add_and_commit(&repo, "changed.txt", "before\n", "add changed");

        fs::write(dir.path().join("staged.txt"), "before\nafter\n").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("staged.txt")).unwrap();
            index.write().unwrap();
        }
        fs::write(dir.path().join("changed.txt"), "changed\n").unwrap();
        fs::write(dir.path().join("untracked.txt"), "new\n").unwrap();

        let repo_path = dir.path().to_str().unwrap();
        let expected_status = get_git_status(repo_path).unwrap();
        let expected_diff_stats = get_status_diff_stats(repo_path).unwrap();

        reset_status_walk_count_for_tests();
        let scan = get_repository_status_scan(repo_path).unwrap();

        assert_eq!(scan.status, expected_status);
        assert_eq!(scan.diff_stats, expected_diff_stats);
        assert_eq!(scan.dirty_count, expected_status.len());
        assert_eq!(status_walk_count_for_tests(), 1);
    }

    #[test]
    pub fn test_差分統計_ステージ済み新規ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join("new.txt"), "line1\nline2\nline3\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("new.txt")).unwrap();
        index.write().unwrap();

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        let file = stats.iter().find(|s| s.path == "new.txt").unwrap();
        assert_eq!(file.index_additions, 3);
        assert_eq!(file.index_deletions, 0);
        assert_eq!(file.wt_additions, 0);
        assert_eq!(file.wt_deletions, 0);
    }

    #[test]
    pub fn test_差分統計_作業ツリー変更() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "original\n", "add file");

        fs::write(dir.path().join("file.txt"), "original\nmodified\n").unwrap();

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        let file = stats.iter().find(|s| s.path == "file.txt").unwrap();
        assert_eq!(file.index_additions, 0);
        assert_eq!(file.index_deletions, 0);
        assert_eq!(file.wt_additions, 1);
        assert_eq!(file.wt_deletions, 0);
    }

    #[test]
    pub fn test_差分統計_ステージと作業ツリー両方() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\n", "add file");

        fs::write(dir.path().join("file.txt"), "line1\nline2\nline3\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.write().unwrap();

        fs::write(dir.path().join("file.txt"), "line1\nline2\nline3\nline4\n").unwrap();

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        let file = stats.iter().find(|s| s.path == "file.txt").unwrap();
        assert_eq!(file.index_additions, 1); // line3 staged
        assert_eq!(file.index_deletions, 0);
        assert_eq!(file.wt_additions, 1); // line4 unstaged
        assert_eq!(file.wt_deletions, 0);
    }

    #[test]
    pub fn test_差分統計_未追跡新規ファイル() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        fs::write(dir.path().join("new.txt"), "line1\nline2\n").unwrap();

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        let file = stats.iter().find(|s| s.path == "new.txt").unwrap();
        assert_eq!(file.index_additions, 0);
        assert_eq!(file.index_deletions, 0);
        assert_eq!(file.wt_additions, 2);
        assert_eq!(file.wt_deletions, 0);
    }

    #[test]
    pub fn test_差分統計_空() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let stats = get_status_diff_stats(dir.path().to_str().unwrap()).unwrap();
        assert!(stats.is_empty());
    }
}
