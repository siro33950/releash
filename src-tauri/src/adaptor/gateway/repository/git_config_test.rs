use super::*;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::{create_initial_commit, create_test_repo};
#[test]
fn test_git設定_各操作の停止を既定値に変えず後続へ進まない() {
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
fn test_base解決_各停止点で欠損へ変換せず次の候補へ進まない() {
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
fn test_base読取_設定破損を未設定や既定branchに変換しない() {
    let (dir, repo) = create_test_repo();
    let path = dir.path().to_str().unwrap();
    assert_eq!(get_branch_base(path, "unborn").unwrap(), None);
    create_initial_commit(&repo);
    let path = dir.path().to_str().unwrap();
    let branch = repo.head().unwrap().shorthand().unwrap().to_string();
    assert_eq!(
        get_branch_base(path, &branch).unwrap(),
        Some(branch.clone())
    );
    assert_eq!(get_releash_base(path).unwrap(), None);
    assert!(resolve_effective_base_branch(path).is_ok());
    std::fs::write(repo.path().join("config"), "[broken\n").unwrap();
    assert!(get_branch_base(path, &branch).is_err());
    assert!(get_releash_base(path).is_err());
    assert!(resolve_current_base_branch(path).is_err());
    assert!(resolve_effective_base_branch(path).is_err());
}
