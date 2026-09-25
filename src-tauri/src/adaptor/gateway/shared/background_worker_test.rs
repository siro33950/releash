use super::*;
use crate::usecase::work_queue::{Attempt, WorkQueueRuntime};
use std::time::Duration;

pub(crate) async fn assert_expired_releases(attempt: Attempt<'static>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resource.lock");
    let child = Arc::new(std::sync::Mutex::new(None));
    let blocked = BlockedRequest {
        path: path.clone(),
        child: child.clone(),
    };
    let release_path = path.clone();
    let task = tokio::spawn(async move {
        let runtime = crate::adaptor::gateway::work_queue::TokioWorkQueueRuntime::default();
        BLOCKED_REQUEST
            .scope(
                blocked,
                runtime.attempt(Box::pin(async move {
                    let _release = ReleaseAfterChild(release_path);
                    attempt.await
                })),
            )
            .await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let resource = loop {
        if let Ok(file) = std::fs::File::open(&path) {
            if fs2::FileExt::try_lock_exclusive(&file).is_err() {
                break file;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "worker did not acquire resource"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    let error = task.await.unwrap().unwrap_err();
    tokio::time::resume();
    assert_eq!(error.kind, FailureKind::Expired);
    let child = child.lock().unwrap().take().expect("registered worker");
    let mut child = child.lock().unwrap();
    assert!(
        child.try_wait().unwrap().is_some(),
        "expired worker was not reaped"
    );
    assert!(child.id().is_none());
    fs2::FileExt::try_lock_exclusive(&resource).expect("expired worker retained resource");
}

struct ReleaseAfterChild(PathBuf);
impl Drop for ReleaseAfterChild {
    fn drop(&mut self) {
        let file = std::fs::File::open(&self.0).expect("worker resource");
        fs2::FileExt::try_lock_exclusive(&file)
            .expect("attempt dropped its locks before worker stopped");
    }
}

#[test]
fn test_子プロセス失敗分類_全分類を変更せず往復する() {
    use FailureKind::*;
    // Given
    for kind in [
        Temporary,
        RestartRequired,
        StateRequired,
        InvalidInput,
        Expired,
        Missing,
        AlreadyPresent,
        Permission,
        Capacity,
        Unsupported,
        Internal,
        Corrupt,
        Cancelled,
        Unknown,
        OutsideRange,
        AuthenticationRequired,
    ] {
        let failure = WorkerFailure::from(WorkFailure {
            kind,
            message: "reason".into(),
        });
        // When
        let wire = serde_json::to_vec(&failure).unwrap();
        let decoded: WorkFailure = serde_json::from_slice::<WorkerFailure>(&wire)
            .unwrap()
            .into();
        // Then
        assert_eq!(decoded.kind, kind);
        assert_eq!(decoded.message, "reason");
    }
}
