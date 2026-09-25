use super::*;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::{create_initial_commit, create_test_repo};
#[test]
fn test_差分行数_各操作の停止をゼロ行に変えず後続へ進まない() {
    // Given
    let (dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    crate::test_support::git::add_and_commit(&repo, "file", "before\n", "file");
    std::fs::write(dir.path().join("file"), "after\nextra\n").unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| get_repository_status_scan(dir.path().to_str().unwrap()));
}
