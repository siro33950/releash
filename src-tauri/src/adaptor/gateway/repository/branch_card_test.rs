use super::super::test_helpers::assert_stops_at_each_checkpoint;
use super::*;
use crate::test_support::git::{add_and_commit, create_initial_commit, create_test_repo};

#[test]
fn test_ブランチカード_各git操作の停止を欠損や既定値に変えない() {
    // Given
    let (directory, repo) = create_test_repo();
    create_initial_commit(&repo);
    let initial = repo.head().unwrap().target().unwrap();
    let name = repo.head().unwrap().shorthand().unwrap().to_string();
    repo.branch("base", &repo.find_commit(initial).unwrap(), false)
        .unwrap();
    repo.worktree("linked", &directory.path().join("linked"), None)
        .unwrap();
    let latest = add_and_commit(&repo, "next", "next", "next");
    repo.find_branch(&name, BranchType::Local)
        .unwrap()
        .set_upstream(Some("base"))
        .unwrap();
    repo.config()
        .unwrap()
        .set_str("releash.base", "base")
        .unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| build_worktree_map(&repo));
    assert_stops_at_each_checkpoint(|| {
        compute_is_merged(&repo, initial, Some(latest)).map_err(RepositoryError::from)
    });
    assert_stops_at_each_checkpoint(|| {
        list_branches_with_status(directory.path().to_str().unwrap())
    });
    let wt_map = HashMap::new();
    let config = repo.config().unwrap();
    let context = BranchCardBuildContext {
        repo: &repo,
        wt_map: &wt_map,
        config: Some(&config),
        base_target_oid: Some(initial),
        main_workdir: None,
    };
    let branch = repo.find_branch(&name, BranchType::Local).unwrap();
    assert_stops_at_each_checkpoint(|| {
        build_branch_card(
            &branch,
            name.clone(),
            &context,
            &mut DirtyCountSnapshot::empty(),
        )
    });
    assert_stops_at_each_checkpoint(|| {
        build_unmatched_worktree_card(
            "detached",
            directory.path().to_str().unwrap(),
            None,
            &mut DirtyCountSnapshot::empty(),
        )
    });
}

#[test]
fn test_ブランチカード_通常のgit取得失敗では従来の既定値を保つ() {
    // Given
    let (_directory, repo) = create_test_repo();
    create_initial_commit(&repo);
    let oid = repo.head().unwrap().target().unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("releash.base", "missing").unwrap();
    // When / Then
    assert_eq!(
        resolve_base_target_oid(&repo, Some(&config), Some(oid)).unwrap(),
        Some(oid)
    );
    assert!(!compute_is_merged(&repo, Oid::ZERO_SHA1, Some(oid)).unwrap());
    assert_eq!(
        DirtyCountSnapshot::empty()
            .dirty_count_for_path("/missing/worktree")
            .unwrap(),
        0
    );
}
