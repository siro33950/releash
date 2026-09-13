use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Files(Mutex<Vec<String>>);
impl FileWatchGateway for Files {
    fn start(&self, path: &str) -> Result<u64, String> {
        self.0.lock().unwrap().push(path.into());
        if path == "/missing" {
            Err("missing path".into())
        } else {
            Ok(42)
        }
    }
    fn stop(&self, watcher_id: u64) -> Result<(), String> {
        self.0.lock().unwrap().push(watcher_id.to_string());
        if watcher_id == 42 {
            Ok(())
        } else {
            Err("unknown watcher".into())
        }
    }
}

#[test]
fn test_監視_repository外ではfile_gatewayに引数と結果を委譲する() {
    // Given
    let files = Arc::new(Files::default());
    let usecase = WatcherUsecase::new(None, files.clone());
    // When / Then
    assert_eq!(usecase.start("/file"), Ok(42));
    assert_eq!(usecase.stop(42), Ok(()));
    assert_eq!(
        usecase.start("/missing"),
        Err(UsecaseError::File("missing path".into()))
    );
    assert_eq!(
        usecase.stop(999),
        Err(UsecaseError::File("unknown watcher".into()))
    );
    assert_eq!(*files.0.lock().unwrap(), ["/file", "42", "/missing", "999"]);
    assert!(usecase.start_git_dir("/repo").is_err());
}

#[tokio::test]
async fn test_監視_repositoryの購読と解除ではfile_gatewayを呼ばない() {
    // Given
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let repository = Arc::new(crate::usecase::repository_state::service::tests::watching_service());
    let files = Arc::new(Files::default());
    let usecase = WatcherUsecase::new(Some(repository), files.clone());
    // When / Then
    let file = usecase.start(path).unwrap();
    let git = usecase.start_git_dir(path).unwrap();
    usecase.stop(file).unwrap();
    usecase.stop(git).unwrap();
    assert!(files.0.lock().unwrap().is_empty());
    assert!(usecase.start("/missing-watcher-path").is_err());
    assert!(files.0.lock().unwrap().is_empty());
    assert_eq!(
        usecase.stop(u64::MAX),
        Err(UsecaseError::File("unknown watcher".into()))
    );
}
