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
fn test_base読取_コミットがないbranchは未設定を返す() {
    // Given
    let (dir, _repo) = create_test_repo();
    // When
    let result = get_branch_base(dir.path().to_str().unwrap(), "unborn");
    // Then
    assert_eq!(result.unwrap(), None);
}
#[test]
fn test_base読取_正常な設定から現在branchと既定baseを読む() {
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
fn test_git任意読取_resolve_effective_base_branch_リポジトリでないディレクトリは不在を返す() {
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
fn test_git任意読取_resolve_effective_base_branch_存在しないパスは不在を返す() {
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
