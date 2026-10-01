use super::*;
use crate::adaptor::gateway::shared::git_operation;

use crate::adaptor::gateway::repository::test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support::git::create_test_repo;
#[test]
fn test_origin探索_各操作の停止をremote不在に変えない() {
    // Given
    let (dir, repo) = create_test_repo();
    // When
    let path = dir.path().to_str().unwrap();
    // Then
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
fn test_origin探索_origin不在はgithubでないと返す() {
    // Given
    let (dir, _repo) = create_test_repo();
    let path = dir.path().to_str().unwrap();
    // When
    let origin = get_origin_url(path);
    let github = is_github_repository(path);
    // Then
    assert_eq!(origin.unwrap(), None);
    assert!(!github.unwrap());
}
#[test]
fn test_origin探索_githubでないurlはgithubでないと返す() {
    // Given
    let (dir, repo) = create_test_repo();
    repo.remote("origin", "https://example.com/repo").unwrap();
    // When
    let github = is_github_repository(dir.path().to_str().unwrap());
    // Then
    assert!(!github.unwrap());
}
#[test]
fn test_origin探索_url読取失敗を不在や非githubに変換しない() {
    // Given
    let (dir, repo) = create_test_repo();
    let path = dir.path().to_str().unwrap();
    let config = repo.path().join("config");
    let mut bytes = std::fs::read(&config).unwrap();
    bytes.extend_from_slice(b"\n[remote \"origin\"]\nurl = \xff\n");
    std::fs::write(config, bytes).unwrap();
    // When
    let origin = get_origin_url(path);
    let github = is_github_repository(path);
    // Then
    assert!(origin.is_err());
    assert!(github.is_err());
}

#[test]
fn test_git任意読取_get_origin_url_リポジトリでないディレクトリは不在を返す() {
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
    let result = get_origin_url(path.to_str().unwrap());
    // Then
    assert_eq!(result.unwrap(), None);
}

#[test]
fn test_git任意読取_get_origin_url_存在しないパスは不在を返す() {
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
    let result = get_origin_url(path.to_str().unwrap());
    // Then
    assert_eq!(result.unwrap(), None);
}
