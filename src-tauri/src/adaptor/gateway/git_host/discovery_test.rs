use super::*;
use crate::adaptor::gateway::shared::git_operation;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::create_test_repo;
#[test]
fn test_origin探索_各操作の停止をremote不在に変えない() {
    // Given
    let (dir, repo) = create_test_repo();
    let path = dir.path().to_str().unwrap();
    // When / Then
    assert_stops_at_each_checkpoint(|| {
        is_github_repository(path).map_err(|error| match error {
            git_operation::GitOperationError::Stopped(error) => error.into(),
            git_operation::GitOperationError::Git(error) => {
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: error.to_string(),
                }
            }
        })
    });
    repo.remote("origin", "https://github.com/test/repository")
        .unwrap();
    assert_stops_at_each_checkpoint(|| {
        is_github_repository(path).map_err(|error| match error {
            git_operation::GitOperationError::Stopped(error) => error.into(),
            git_operation::GitOperationError::Git(error) => {
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: error.to_string(),
                }
            }
        })
    });
}

#[test]
fn test_origin探索_url読取失敗を不在や非githubに変換しない() {
    let (dir, repo) = create_test_repo();
    let path = dir.path().to_str().unwrap();
    assert_eq!(get_origin_url(path).unwrap(), None);
    assert!(!is_github_repository(path).unwrap());
    repo.remote("origin", "https://example.com/repo").unwrap();
    assert!(!is_github_repository(path).unwrap());
    let config = repo.path().join("config");
    let mut bytes = std::fs::read(&config).unwrap();
    bytes.extend_from_slice(b"\n[remote \"origin\"]\nurl = \xff\n");
    std::fs::write(config, bytes).unwrap();
    assert!(get_origin_url(path).is_err());
    assert!(is_github_repository(path).is_err());
}
