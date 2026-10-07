use releashd::test_support::integration::fixtures::watcher_Files as Files;
use releashd::test_support::integration::platform::watcher_UsecaseError as UsecaseError;
use releashd::test_support::integration::platform::WatcherUsecase;
use std::sync::Arc;

#[tokio::test]
pub async fn test_監視_repositoryのgit監視の開始と停止ではfile_gatewayを呼ばない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let repository = Arc::new(crate::usecase_repository_state_service::tests::watching_service());
    let files = Arc::new(Files::default());
    let usecase = WatcherUsecase::new(Some(repository), files.clone());
    // When / Then
    let git = usecase.start_git_dir(path).unwrap();
    usecase.stop(git).unwrap();
    assert!(files.0.lock().unwrap().is_empty());
    assert!(
        matches!(usecase.stop(u64::MAX), Err(UsecaseError::File(message)) if message == "unknown watcher")
    );
}
