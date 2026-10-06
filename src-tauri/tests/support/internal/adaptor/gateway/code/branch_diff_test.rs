use super::*;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::{create_initial_commit, create_test_repo};

#[test]
pub fn test_ブランチ差分_各操作の停止を空の差分に変えない() {
    // Given
    let (dir, repo) = create_test_repo();
    let oid = create_initial_commit(&repo).to_string();
    std::fs::write(dir.path().join("file"), "one\ntwo\n").unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| {
        get_branch_diff_summary(dir.path().to_str().unwrap(), Some("main"), Some(&oid))
    });
}
pub(crate) mod branch_diff_gateway_tests {
    use super::super::*;
    use crate::test_support::git::*;
    use git2::build::CheckoutBuilder;

    fn repo_path_str(repo: &Repository) -> String {
        repo.workdir().unwrap().to_str().unwrap().to_string()
    }

    /// feature ブランチを作成・チェックアウトし、分岐元（base）の (ブランチ名, コミット OID)
    /// を返す。base 名解決と ref→OID 解決は repository ドメインの責務に移ったため、gateway の
    /// merge-base 計算テストは解決済みの base 名タグと base コミット OID を明示的に渡す。
    fn setup_feature_branch(repo: &Repository) -> (String, String) {
        let base = repo.head().unwrap().shorthand().unwrap().to_string();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let base_oid = head.id().to_string();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        (base, base_oid)
    }

    #[test]
    pub fn test_branch_diff_単一変更ファイル() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "line1\nline2\nline3\n", "add file");

        let (base, base_oid) = setup_feature_branch(&repo);
        add_and_commit(
            &repo,
            "file.txt",
            "line1\nmodified\nline3\n",
            "modify line2",
        );

        let summary =
            get_branch_diff_summary(&repo_path_str(&repo), Some(&base), Some(&base_oid)).unwrap();
        assert_eq!(summary.changed_files.len(), 1);
        assert_eq!(summary.changed_files[0].path, "file.txt");
        assert_eq!(summary.changed_files[0].status, "modified");
        assert!(!summary.changed_files[0].binary);
    }

    #[test]
    pub fn test_branch_diff_追加ファイル() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "existing.txt", "content\n", "add existing");

        let (base, base_oid) = setup_feature_branch(&repo);
        add_and_commit(&repo, "new.txt", "new\n", "add new file");

        let summary =
            get_branch_diff_summary(&repo_path_str(&repo), Some(&base), Some(&base_oid)).unwrap();
        let added = summary
            .changed_files
            .iter()
            .find(|f| f.path == "new.txt")
            .expect("new.txt should be in diff");
        assert_eq!(added.status, "added");
    }

    #[test]
    pub fn test_branch_diff_変更なし() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content\n", "add file");

        let (base, base_oid) = setup_feature_branch(&repo);
        let summary =
            get_branch_diff_summary(&repo_path_str(&repo), Some(&base), Some(&base_oid)).unwrap();
        assert!(summary.changed_files.is_empty());
        assert_eq!(summary.stats.additions, 0);
        assert_eq!(summary.stats.deletions, 0);
    }

    #[test]
    pub fn test_branch_diff_未追跡ファイルを含む() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "existing.txt", "content\n", "add existing");

        let (base, base_oid) = setup_feature_branch(&repo);
        let workdir = repo.workdir().unwrap();
        std::fs::write(workdir.join("untracked.txt"), "hello\n").unwrap();

        let summary =
            get_branch_diff_summary(&repo_path_str(&repo), Some(&base), Some(&base_oid)).unwrap();
        let untracked = summary
            .changed_files
            .iter()
            .find(|f| f.path == "untracked.txt")
            .expect("untracked.txt should be included in the branch diff");
        assert_eq!(untracked.status, "added");
    }

    #[test]
    pub fn test_branch_diff_unborn_branchは空() {
        let (_dir, repo) = create_test_repo();
        let summary = get_branch_diff_summary(&repo_path_str(&repo), None, None).unwrap();
        assert!(summary.changed_files.is_empty());
        assert_eq!(summary.stats.additions, 0);
        assert_eq!(summary.stats.deletions, 0);
        assert_eq!(summary.base_branch, "");
    }

    #[test]
    pub fn test_branch_diff_base未指定_headフォールバック() {
        // base 名が None（detached / base 未設定）の場合、merge-base ではなく HEAD と
        // workdir の差分になる。ここではコミット済みのため変更なし、untracked のみ検出される。
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content\n", "add file");
        let workdir = repo.workdir().unwrap();
        std::fs::write(workdir.join("untracked.txt"), "hello\n").unwrap();

        let summary = get_branch_diff_summary(&repo_path_str(&repo), None, None).unwrap();
        assert_eq!(summary.base_branch, "HEAD");
        assert!(summary
            .changed_files
            .iter()
            .any(|f| f.path == "untracked.txt"));
        assert!(!summary.changed_files.iter().any(|f| f.path == "file.txt"));
    }
}
