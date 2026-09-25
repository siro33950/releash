use super::*;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::{create_initial_commit, create_test_repo};
#[test]
fn test_base解決_各候補の停止で次候補へ進まない() {
    // Given
    let (_dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let config = repo.config().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| resolve_branch_base(&repo, Some(&config), "feature"));
}
