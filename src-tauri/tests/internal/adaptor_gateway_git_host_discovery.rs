use releashd::test_support::integration::platform::get_origin_url;
use releashd::test_support::integration::platform::is_github_repository;

use crate::adaptor_gateway_repository_test_helpers::assert_stops_at_each_checkpoint;
use crate::test_support_git::create_test_repo;

#[test]
pub fn test_origin探索_各操作の停止をremote不在に変えない() {
    // Given
    let (dir, repo) = create_test_repo();
    // When
    let path = dir.path().to_str().unwrap();
    // Then
    assert_stops_at_each_checkpoint(|| {
        is_github_repository(path).map_err(|error| match error {
            releashd::test_support::integration::platform::GitOperationError::Stopped(error) => error.into(),
            releashd::test_support::integration::platform::GitOperationError::Git(error) => {
                releashd::test_support::integration::platform::TechnicalFailure {
                    nature: releashd::test_support::integration::platform::TechnicalFailureNature::Other,
                    message: error.to_string(),
                }
            }
        })
    });
    repo.remote("origin", "https://github.com/test/repository")
        .unwrap();
    assert_stops_at_each_checkpoint(|| {
        is_github_repository(path).map_err(|error| match error {
            releashd::test_support::integration::platform::GitOperationError::Stopped(error) => error.into(),
            releashd::test_support::integration::platform::GitOperationError::Git(error) => {
                releashd::test_support::integration::platform::TechnicalFailure {
                    nature: releashd::test_support::integration::platform::TechnicalFailureNature::Other,
                    message: error.to_string(),
                }
            }
        })
    });
}

#[test]
pub fn test_origin探索_origin不在はgithubでないと返す() {
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
pub fn test_origin探索_githubでないurlはgithubでないと返す() {
    // Given
    let (dir, repo) = create_test_repo();
    repo.remote("origin", "https://example.com/repo").unwrap();
    // When
    let github = is_github_repository(dir.path().to_str().unwrap());
    // Then
    assert!(!github.unwrap());
}

#[test]
pub fn test_origin探索_url読取失敗を不在や非githubに変換しない() {
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
pub fn test_git任意読取_get_origin_url_リポジトリでないディレクトリは不在を返す() {
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
pub fn test_git任意読取_get_origin_url_存在しないパスは不在を返す() {
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
pub(crate) mod tests {

    use releashd::test_support::integration::platform::get_origin_url;
    use releashd::test_support::integration::platform::is_github_repository;

    #[test]
    pub fn get_origin_url_no_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        assert!(get_origin_url(dir.path().to_str().unwrap())
            .unwrap()
            .is_none());
    }

    #[test]
    pub fn get_origin_url_with_github_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "https://github.com/user/repo.git")
            .unwrap();

        let url = get_origin_url(dir.path().to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(url.contains("github.com"));
    }

    #[test]
    pub fn is_github_repository_returns_false_for_no_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        git2::Repository::init(dir.path()).unwrap();

        assert!(!is_github_repository(dir.path().to_str().unwrap()).unwrap());
    }

    #[test]
    pub fn is_github_repository_returns_true_for_github_remote() {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        repo.remote("origin", "git@github.com:user/repo.git")
            .unwrap();

        assert!(is_github_repository(dir.path().to_str().unwrap()).unwrap());
    }
}
