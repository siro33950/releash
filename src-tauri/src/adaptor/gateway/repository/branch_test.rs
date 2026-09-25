use super::*;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::{create_initial_commit, create_test_repo};
#[test]
fn test_ブランチ参照_各操作で停止を保持する() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = dir.path().to_str().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| list_branches(path));
    assert_stops_at_each_checkpoint(|| get_current_branch(path));
    assert_stops_at_each_checkpoint(|| get_default_branch(path));
}
#[test]
fn test_ブランチ変更_各操作の停止で後続へ進まない() {
    // Given / When / Then
    assert_stops_at_each_checkpoint(|| {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        git_create_branch(dir.path().to_str().unwrap(), "feature")
    });
    assert_stops_at_each_checkpoint(|| {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        repo.branch(
            "feature",
            &repo.head().unwrap().peel_to_commit().unwrap(),
            false,
        )
        .unwrap();
        delete_branch(dir.path().to_str().unwrap(), "feature")
    });
}
