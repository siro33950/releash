use releash_lib::test_support::integration::repository::resolve_branch_base;

use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support_git::create_initial_commit;
use crate::test_support_git::create_test_repo;

#[test]
pub fn test_base解決_各候補の停止で次候補へ進まない() {
    // Given
    let (_dir, repo) = create_test_repo();
    create_initial_commit(&repo);
    let config = repo.config().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| {
        resolve_branch_base(&repo, Some(&config), "feature")
            .map_err(releash_lib::test_support::integration::platform::TechnicalFailure::from)
    });
}
