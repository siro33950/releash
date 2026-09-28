use super::*;
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use std::sync::Mutex;

#[derive(Default)]
struct RecordingWatch {
    calls: Mutex<Vec<&'static str>>,
    start_failures: Mutex<usize>,
}

#[async_trait::async_trait]
impl ReviewCommentsWatch for RecordingWatch {
    async fn ensure_started(&self) -> Result<(), TechnicalFailure> {
        self.calls.lock().unwrap().push("start");
        let mut failures = self.start_failures.lock().unwrap();
        if *failures > 0 {
            *failures -= 1;
            return Err(TechnicalFailure {
                nature: TechnicalFailureNature::Transient,
                message: "busy".into(),
            });
        }
        Ok(())
    }
    async fn poll(&self) -> Result<(), TechnicalFailure> {
        self.calls.lock().unwrap().push("poll");
        Ok(())
    }
    async fn restart(&self) -> Result<(), TechnicalFailure> {
        self.calls.lock().unwrap().push("restart");
        Ok(())
    }
}

#[tokio::test]
async fn test_review監視_続行では開始を確かめてから読み取り読み直しでは作り直す() {
    // Given
    let watch = Arc::new(RecordingWatch::default());
    let usecase = ReviewCommentsWatchUsecase::new(watch.clone());
    // When
    usecase.poll(AttemptProgress::Continue).await.unwrap();
    usecase.poll(AttemptProgress::Reload).await.unwrap();
    // Then
    assert_eq!(
        *watch.calls.lock().unwrap(),
        ["start", "poll", "restart", "start", "poll"]
    );
}

#[tokio::test]
async fn test_review監視_開始の失敗を技術的な失敗の性質ごと返す() {
    let watch = Arc::new(RecordingWatch::default());
    *watch.start_failures.lock().unwrap() = 1;
    let usecase = ReviewCommentsWatchUsecase::new(watch.clone());
    let error = usecase.poll(AttemptProgress::Continue).await.unwrap_err();
    assert_eq!(
        error.kind,
        crate::usecase::failure::Failure::Technical(TechnicalFailureNature::Transient)
    );
    assert_eq!(error.message, "busy");
    assert_eq!(*watch.calls.lock().unwrap(), ["start"]);
}
