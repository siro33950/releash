use super::*;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::{create_initial_commit, create_test_repo};
#[test]
fn test_merge_base解決_各操作の停止で後続へ進まない() {
    // Given
    let (_dir, repo) = create_test_repo();
    let oid = create_initial_commit(&repo).to_string();
    // When / Then
    for base in [None, Some(oid.as_str())] {
        assert_stops_at_each_checkpoint(|| resolve_merge_base_commit(&repo, base));
    }
}
