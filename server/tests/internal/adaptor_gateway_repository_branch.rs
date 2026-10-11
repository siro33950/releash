use releashd::test_support::integration::repository::get_current_branch;
use releashd::test_support::integration::repository::git_create_branch;
use releashd::test_support::integration::repository::list_branches;

use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support_git::create_initial_commit;
use crate::test_support_git::create_test_repo;

#[test]
pub fn test_ブランチ参照_各操作で停止を保持する() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = dir.path().to_str().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| list_branches(path));
    assert_stops_at_each_checkpoint(|| get_current_branch(path));
}

#[test]
pub fn test_ブランチ変更_各操作の停止で後続へ進まない() {
    // Given / When / Then
    assert_stops_at_each_checkpoint(|| {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        git_create_branch(dir.path().to_str().unwrap(), "feature")
    });
}
pub(crate) mod branch_gateway_tests {
    use super::*;

    use git2::build::CheckoutBuilder;
    use git2::BranchType;
    use releashd::test_support::integration::platform::detect_default_branch;
    use releashd::test_support::integration::repository::get_current_branch;
    use releashd::test_support::integration::repository::git_create_branch;

    use std::path::Path;

    fn path_str(p: &Path) -> String {
        p.to_str().unwrap().to_string()
    }

    #[test]
    pub fn test_現在ブランチ取得_初期コミット後() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let result = get_current_branch(&path_str(dir.path())).unwrap();
        assert!(result == "main" || result == "master");
    }

    #[test]
    pub fn test_現在ブランチ取得_空リポジトリ() {
        let (dir, _repo) = create_test_repo();

        let result = get_current_branch(&path_str(dir.path())).unwrap();
        assert_eq!(result, "(no commits)");
    }

    #[test]
    pub fn test_現在ブランチ取得_detached_head() {
        let (dir, repo) = create_test_repo();
        let oid = create_initial_commit(&repo);

        repo.set_head_detached(oid).unwrap();

        let result = get_current_branch(&path_str(dir.path())).unwrap();
        assert!(result.starts_with('('));
        assert!(result.ends_with(')'));
        assert_eq!(result.len(), 9); // "(1234567)"
    }

    #[test]
    pub fn test_ブランチ作成() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        git_create_branch(&path_str(dir.path()), "feature").unwrap();

        let branch = get_current_branch(&path_str(dir.path())).unwrap();
        assert_eq!(branch, "feature");
    }

    #[test]
    pub fn test_ブランチ作成_既存名でエラー() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        git_create_branch(&path_str(dir.path()), "feature").unwrap();

        let result = git_create_branch(&path_str(dir.path()), "feature");
        assert!(result.is_err());
    }

    #[test]
    pub fn test_既定ブランチ取得() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let branch = detect_default_branch(&repo).unwrap().unwrap();
        assert!(
            branch == "main" || branch == "master",
            "expected main or master, got {branch}"
        );

        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        let other = if branch == "main" { "master" } else { "main" };
        repo.branch(other, &head_commit, false).unwrap();
        repo.set_head(&format!("refs/heads/{other}")).unwrap();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .unwrap();
        let mut old_branch = repo.find_branch(&branch, BranchType::Local).unwrap();
        old_branch.delete().unwrap();

        let new_default = detect_default_branch(&repo).unwrap().unwrap();
        assert_eq!(new_default, other);
    }

    #[test]
    pub fn test_既定ブランチ検出_remote_head() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("develop", &head, false).unwrap();

        repo.reference(
            "refs/remotes/origin/develop",
            head.id(),
            true,
            "test remote branch",
        )
        .unwrap();
        repo.reference_symbolic(
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/develop",
            true,
            "test remote HEAD",
        )
        .unwrap();

        let default_name = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().ok().map(|s| s.to_string()));
        if let Some(name) = default_name {
            if name == "main" || name == "master" {
                repo.set_head("refs/heads/develop").unwrap();
                repo.checkout_head(Some(CheckoutBuilder::new().force()))
                    .unwrap();
                let mut b = repo.find_branch(&name, BranchType::Local).unwrap();
                b.delete().unwrap();
            }
        }

        let result = detect_default_branch(&repo).unwrap();
        assert_eq!(result, Some("develop".to_string()));
    }
}
