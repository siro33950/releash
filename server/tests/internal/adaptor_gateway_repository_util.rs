use releashd::test_support::integration::repository::RepositoryConfigRepository;
use releashd::test_support::integration::settings::AppConfig;

use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support_git::{create_initial_commit, create_test_repo};

#[test]
pub fn test_base解決_各候補の停止で次候補へ進まない() {
    // Given
    let (directory, repo) = create_test_repo();
    create_initial_commit(&repo);
    let path = directory.path().to_str().unwrap();
    let settings = AppConfig::new(Default::default(), std::path::PathBuf::new());
    let mut config = repo.config().unwrap();

    // When / Then
    for key in [
        None,
        Some("releash.base"),
        Some("branch.feature.releash-base"),
    ] {
        if let Some(key) = key {
            config.set_str(key, "main").unwrap();
        }
        assert_stops_at_each_checkpoint(|| settings.load(path));
    }
}
