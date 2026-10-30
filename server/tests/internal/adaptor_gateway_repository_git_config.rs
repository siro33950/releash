use releashd::test_support::integration::repository::resolve_effective_base_branch;

use crate::test_support_git::create_initial_commit;
use crate::test_support_git::create_test_repo;

#[test]
pub fn test_git設定_各操作の停止を既定値に変えず後続へ進まない() {
    todo!()
}

#[test]
pub fn test_base解決_各停止点で欠損へ変換せず次の候補へ進まない() {
    todo!()
}

#[test]
pub fn test_base読取_設定破損を未設定や既定branchに変換しない() {
    todo!()
}

#[test]
pub fn test_base読取_コミットがないbranchは未設定を返す() {
    todo!()
}

#[test]
pub fn test_base読取_正常な設定から現在branchと既定baseを読む() {
    todo!()
}

#[test]
pub fn test_git任意読取_resolve_effective_base_branch_リポジトリでないディレクトリは不在を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        git2::Repository::discover(directory.path())
            .err()
            .unwrap()
            .code(),
        git2::ErrorCode::NotFound
    );
    let path = directory.path().to_path_buf();
    // When
    let result = resolve_effective_base_branch(path.to_str().unwrap());
    // Then
    assert_eq!(result.unwrap(), None);
}

#[test]
pub fn test_git任意読取_resolve_effective_base_branch_存在しないパスは不在を返す() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        git2::Repository::discover(directory.path())
            .err()
            .unwrap()
            .code(),
        git2::ErrorCode::NotFound
    );
    let path = directory.path().join("missing");
    // When
    let result = resolve_effective_base_branch(path.to_str().unwrap());
    // Then
    assert_eq!(result.unwrap(), None);
}
pub(crate) mod git_config_gateway_tests {
    use super::*;

    use releashd::test_support::integration::repository::get_releash_base;

    use releashd::test_support::integration::repository::resolve_effective_base_branch;
    use releashd::test_support::integration::repository::set_branch_base_override;
    use releashd::test_support::integration::repository::set_releash_base;

    #[test]
    pub fn test_ベース解決_per_branch() {
        todo!()
    }

    #[test]
    pub fn test_ベース解決_releash_baseフォールバック() {
        todo!()
    }

    #[test]
    pub fn test_ベース解決_既定ブランチフォールバック() {
        todo!()
    }

    #[test]
    pub fn test_branch_base_取得設定() {
        todo!()
    }

    #[test]
    pub fn test_releash_base_取得設定() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let repo_path = dir.path().to_str().unwrap();

        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, None);

        set_releash_base(repo_path, Some("develop")).unwrap();
        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, Some("develop".to_string()));

        set_releash_base(repo_path, None).unwrap();
        let base = get_releash_base(repo_path).unwrap();
        assert_eq!(base, None);
    }

    fn checkout_feature_branch(repo: &git2::Repository) {
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("feature", &head, false).unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
    }

    #[test]
    pub fn test_現在ブランチbase解決_override優先() {
        todo!()
    }

    #[test]
    pub fn test_現在ブランチbase解決_detached_none() {
        todo!()
    }

    #[test]
    pub fn test_実効base_ref実在ならsome_ref不在ならnone() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        let default_branch = repo.head().unwrap().shorthand().unwrap().to_string();

        checkout_feature_branch(&repo);
        set_branch_base_override(repo_path, "feature", Some(&default_branch)).unwrap();
        assert_eq!(
            resolve_effective_base_branch(repo_path).unwrap(),
            Some(default_branch)
        );

        set_branch_base_override(repo_path, "feature", Some("no-such-branch")).unwrap();
        assert_eq!(resolve_effective_base_branch(repo_path).unwrap(), None);
    }

    #[test]
    pub fn test_実効base_merge_base不成立ならnone() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        let default_branch = repo.head().unwrap().shorthand().unwrap().to_string();
        let signature = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let orphan = repo
            .commit(None, &signature, &signature, "orphan root", &tree, &[])
            .unwrap();
        repo.reference("refs/heads/feature", orphan, true, "orphan")
            .unwrap();
        repo.set_head("refs/heads/feature").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        set_branch_base_override(repo_path, "feature", Some(&default_branch)).unwrap();

        assert_eq!(resolve_effective_base_branch(repo_path).unwrap(), None);
    }

    #[test]
    pub fn test_gc_現存しないブランチのbaseを掃除() {
        todo!()
    }
}

#[test]
fn test_git設定_親ディレクトリごと削除されたファイルでもbaseを解決する() {
    todo!()
}
