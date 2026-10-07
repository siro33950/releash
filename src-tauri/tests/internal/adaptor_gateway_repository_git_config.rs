use releashd::test_support::integration::repository::get_branch_base;
use releashd::test_support::integration::repository::get_releash_base;
use releashd::test_support::integration::repository::prune_stale_branch_bases;
use releashd::test_support::integration::repository::resolve_base_commit_oid;
use releashd::test_support::integration::repository::resolve_current_base_branch;
use releashd::test_support::integration::repository::resolve_effective_base_branch;
use releashd::test_support::integration::repository::set_branch_base_override;
use releashd::test_support::integration::repository::set_releash_base;

use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support_git::create_initial_commit;
use crate::test_support_git::create_test_repo;

#[test]
pub fn test_git設定_各操作の停止を既定値に変えず後続へ進まない() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = dir.path().to_str().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| get_branch_base(path, "main"));
    assert_stops_at_each_checkpoint(|| get_releash_base(path));
    assert_stops_at_each_checkpoint(|| set_releash_base(path, Some("main")));
    assert_stops_at_each_checkpoint(|| set_branch_base_override(path, "stale", Some("main")));
    assert_stops_at_each_checkpoint(|| {
        git2::Repository::open(path)
            .unwrap()
            .config()
            .unwrap()
            .set_str("branch.stale.releash-base", "main")
            .unwrap();
        prune_stale_branch_bases(path, &[])
    });
}

#[test]
pub fn test_base解決_各停止点で欠損へ変換せず次の候補へ進まない() {
    // Given
    let (dir, repo) = create_test_repo();
    let oid = create_initial_commit(&repo);
    let path = dir.path().to_str().unwrap();
    repo.config()
        .unwrap()
        .set_str("releash.base", "base")
        .unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| resolve_current_base_branch(path));
    assert_stops_at_each_checkpoint(|| resolve_base_commit_oid(path, "missing"));
    assert_stops_at_each_checkpoint(|| resolve_effective_base_branch(path));
    repo.reference("refs/remotes/origin/base", oid, true, "test")
        .unwrap();
    assert_stops_at_each_checkpoint(|| resolve_base_commit_oid(path, "base"));
    assert_stops_at_each_checkpoint(|| resolve_effective_base_branch(path));
    repo.branch("base", &repo.find_commit(oid).unwrap(), false)
        .unwrap();
    assert_stops_at_each_checkpoint(|| resolve_base_commit_oid(path, "base"));
    assert_stops_at_each_checkpoint(|| resolve_effective_base_branch(path));
    let deleted = dir.path().join("deleted");
    assert_stops_at_each_checkpoint(|| resolve_current_base_branch(deleted.to_str().unwrap()));
}

#[test]
pub fn test_base読取_設定破損を未設定や既定branchに変換しない() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    std::fs::write(repo.path().join("config"), "[broken\n").unwrap();
    let path = dir.path().to_str().unwrap();
    // When
    let branch_base = get_branch_base(path, &branch);
    let releash_base = get_releash_base(path);
    let current_base = resolve_current_base_branch(path);
    let effective_base = resolve_effective_base_branch(path);
    // Then
    assert!(branch_base.is_err());
    assert!(releash_base.is_err());
    assert!(current_base.is_err());
    assert!(effective_base.is_err());
}

#[test]
pub fn test_base読取_コミットがないbranchは未設定を返す() {
    // Given
    let (dir, _repo) = create_test_repo();
    // When
    let result = get_branch_base(dir.path().to_str().unwrap(), "unborn");
    // Then
    assert_eq!(result.unwrap(), None);
}

#[test]
pub fn test_base読取_正常な設定から現在branchと既定baseを読む() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = dir.path().to_str().unwrap();
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    // When
    let branch_base = get_branch_base(path, &branch);
    let releash_base = get_releash_base(path);
    let effective_base = resolve_effective_base_branch(path);
    // Then
    assert_eq!(branch_base.unwrap(), Some(branch));
    assert_eq!(releash_base.unwrap(), None);
    assert!(effective_base.is_ok());
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

    use releashd::test_support::integration::repository::get_branch_base;
    use releashd::test_support::integration::repository::get_releash_base;
    use releashd::test_support::integration::repository::prune_stale_branch_bases;
    use releashd::test_support::integration::repository::resolve_branch_base;
    use releashd::test_support::integration::repository::resolve_current_base_branch;
    use releashd::test_support::integration::repository::resolve_effective_base_branch;
    use releashd::test_support::integration::repository::set_branch_base_override;
    use releashd::test_support::integration::repository::set_releash_base;

    #[test]
    pub fn test_ベース解決_per_branch() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        set_branch_base_override(repo_path, "feat", Some("develop")).unwrap();
        let config = repo.config().ok();
        let result = resolve_branch_base(&repo, config.as_ref(), "feat").unwrap();
        assert_eq!(result, Some("develop".to_string()));
    }

    #[test]
    pub fn test_ベース解決_releash_baseフォールバック() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        set_releash_base(repo_path, Some("develop")).unwrap();
        let config = repo.config().ok();
        let result = resolve_branch_base(&repo, config.as_ref(), "feat").unwrap();
        assert_eq!(result, Some("develop".to_string()));
    }

    #[test]
    pub fn test_ベース解決_既定ブランチフォールバック() {
        let (_dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let config = repo.config().ok();
        let result = resolve_branch_base(&repo, config.as_ref(), "feat").unwrap();
        assert!(result.is_some());
    }

    #[test]
    pub fn test_branch_base_取得設定() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        let base = get_branch_base(repo_path, "feat").unwrap();
        assert!(base.is_some());

        set_branch_base_override(repo_path, "feat", Some("develop")).unwrap();
        let base = get_branch_base(repo_path, "feat").unwrap();
        assert_eq!(base, Some("develop".to_string()));

        set_branch_base_override(repo_path, "feat", None).unwrap();
        let base = get_branch_base(repo_path, "feat").unwrap();
        assert!(base.is_some());
        assert_ne!(base, Some("develop".to_string()));
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
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        let default_branch = repo.head().unwrap().shorthand().unwrap().to_string();

        checkout_feature_branch(&repo);
        set_branch_base_override(repo_path, "feature", Some(&default_branch)).unwrap();

        let result = resolve_current_base_branch(repo_path).unwrap();
        assert_eq!(result, Some(default_branch));
    }

    #[test]
    pub fn test_現在ブランチbase解決_detached_none() {
        let (dir, repo) = create_test_repo();
        let oid = create_initial_commit(&repo);
        repo.set_head_detached(oid).unwrap();

        let result = resolve_current_base_branch(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(result, None);
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
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();

        set_branch_base_override(repo_path, "alive", Some("main")).unwrap();
        set_branch_base_override(repo_path, "stale", Some("main")).unwrap();

        // "alive" のみ現存ブランチとして渡すと "stale" の base が掃除される
        prune_stale_branch_bases(repo_path, &["alive".to_string()]).unwrap();

        let config = repo.config().unwrap();
        assert!(config.get_string("branch.alive.releash-base").is_ok());
        assert!(config.get_string("branch.stale.releash-base").is_err());
    }
}
