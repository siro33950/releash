use super::*;
use crate::domain::repository::file_watcher::WatchChangeHandler;
use std::sync::Mutex;

#[derive(Default)]
struct Files(Mutex<Vec<String>>);
impl FileWatchGateway for Files {
    fn start_tree(&self, path: &str, _on_change: WatchChangeHandler) -> Result<u64, String> {
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

fn ignore() -> WatchChangeHandler {
    Arc::new(|_| {})
}

#[test]
fn test_監視_repository外ではfile_gatewayに引数と結果を委譲する() {
    // Given
    let files = Arc::new(Files::default());
    let usecase = WatcherUsecase::new(None, files.clone());
    // When / Then
    assert_eq!(usecase.start_files("/file", ignore()).unwrap(), 42);
    usecase.stop(42).unwrap();
    assert!(
        matches!(usecase.start_files("/missing", ignore()), Err(UsecaseError::File(message)) if message == "missing path")
    );
    assert!(
        matches!(usecase.stop(999), Err(UsecaseError::File(message)) if message == "unknown watcher")
    );
    assert_eq!(*files.0.lock().unwrap(), ["/file", "42", "/missing", "999"]);
    assert!(matches!(
        usecase.start_git_dir("/repo"),
        Err(UsecaseError::RepositoryUnavailable)
    ));
}

#[test]
fn test_監視_repositoryの再走査競合と下位エラーの分類を保持する() {
    use crate::usecase::repository_state::RepositoryStateError;

    // Given
    for source in [
        RepositoryStateError::ScanInvalidated,
        RepositoryStateError::Repository(
            crate::domain::repository::RepositoryError::rule("state").into(),
        ),
        RepositoryStateError::Code(
            crate::domain::code::CodeError::StaleReviewGroupTarget {
                group_id: "g".into(),
            }
            .into(),
        ),
    ] {
        let expected = format!("{source:?}");
        let message = source.to_string();
        // When
        let error = UsecaseError::Repository(source);
        // Then
        assert!(
            matches!(&error, UsecaseError::Repository(actual) if format!("{actual:?}") == expected)
        );
        assert_eq!(error.to_string(), message);
    }
}
