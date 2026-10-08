use releashd::test_support::integration::platform::with_blocked_request;
use releashd::test_support::integration::platform::BlockedRequest;
use releashd::test_support::integration::platform::Failure;
use releashd::test_support::integration::platform::TechnicalFailureNature;
use releashd::test_support::integration::platform::WorkFailure;
use std::path::PathBuf;
use std::sync::Arc;

use std::time::Duration;

pub(crate) async fn assert_expired_releases<T: Send + std::fmt::Debug + 'static>(
    attempt: impl std::future::Future<Output = Result<T, WorkFailure>> + Send + 'static,
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resource.lock");
    let child = Arc::new(std::sync::Mutex::new(None));
    let blocked = BlockedRequest::new(path.clone(), child.clone());
    let release_path = path.clone();
    let task = tokio::spawn(async move {
        with_blocked_request(blocked, async move {
            let _release = ReleaseAfterChild(release_path);
            attempt.await
        })
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
    assert_eq!(
        error.kind,
        Failure::Technical(TechnicalFailureNature::TimedOut)
    );
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
