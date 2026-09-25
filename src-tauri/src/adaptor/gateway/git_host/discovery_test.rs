use super::*;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::create_test_repo;
#[test]
fn test_origin探索_各操作の停止をremote不在に変えない() {
    // Given
    let (dir, repo) = create_test_repo();
    let path = dir.path().to_str().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| is_github_repository(path));
    repo.remote("origin", "https://github.com/test/repository")
        .unwrap();
    assert_stops_at_each_checkpoint(|| is_github_repository(path));
}
