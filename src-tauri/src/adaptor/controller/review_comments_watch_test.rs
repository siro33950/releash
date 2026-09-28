use super::*;
use crate::domain::comment::ReviewCommentsWatch;
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Watch {
    polls: AtomicUsize,
    failures: std::sync::Mutex<Vec<TechnicalFailureNature>>,
}

#[async_trait::async_trait]
impl ReviewCommentsWatch for Watch {
    async fn ensure_started(&self) -> Result<(), TechnicalFailure> {
        Ok(())
    }
    async fn poll(&self) -> Result<(), TechnicalFailure> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        match self.failures.lock().unwrap().pop() {
            Some(nature) => Err(TechnicalFailure {
                nature,
                message: "poll failed".into(),
            }),
            None => Ok(()),
        }
    }
    async fn restart(&self) -> Result<(), TechnicalFailure> {
        Ok(())
    }
}

async fn run_ticks(watch: Arc<Watch>, ticks: usize) -> Arc<Retrying> {
    let retrying = crate::usecase::retry::test_retrying();
    run(
        retrying.clone(),
        Arc::new(ReviewCommentsWatchUsecase::new(watch)),
        "comments".into(),
        Box::pin(futures_util::stream::iter(vec![(); ticks])),
    )
    .await;
    retrying
}

#[tokio::test(start_paused = true)]
async fn test_review監視_一時的な失敗をやり直してから次の周期へ進む() {
    // Given
    let watch = Arc::new(Watch {
        polls: AtomicUsize::new(0),
        failures: std::sync::Mutex::new(vec![TechnicalFailureNature::Transient]),
    });
    // When
    let retrying = run_ticks(watch.clone(), 2).await;
    // Then
    assert_eq!(watch.polls.load(Ordering::SeqCst), 3);
    let records = retrying.records("comments");
    assert_eq!(records.len(), 1);
    assert!(!records[0].record.active);
}

#[tokio::test(start_paused = true)]
async fn test_review監視_やり直さない失敗で監視を終える() {
    // Given
    let watch = Arc::new(Watch {
        polls: AtomicUsize::new(0),
        failures: std::sync::Mutex::new(vec![TechnicalFailureNature::Other]),
    });
    // When
    let retrying = run_ticks(watch.clone(), 5).await;
    // Then
    assert_eq!(watch.polls.load(Ordering::SeqCst), 1);
    assert!(retrying.records("comments")[0].requires_attention);
}
