use super::*;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::{create_initial_commit, create_test_repo};
#[test]
fn test_ブランチ差分_各操作の停止を空の差分に変えない() {
    // Given
    let (dir, repo) = create_test_repo();
    let oid = create_initial_commit(&repo).to_string();
    std::fs::write(dir.path().join("file"), "one\ntwo\n").unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| {
        get_branch_diff_summary(dir.path().to_str().unwrap(), Some("main"), Some(&oid))
    });
}
