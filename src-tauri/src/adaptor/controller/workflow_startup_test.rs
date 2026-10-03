use super::*;
use crate::domain::failure::{StorageFailureSource, TechnicalFailureNature};
use crate::domain::workflow::repository::{WorkflowStartupRecord, WorkflowStartupRepository};
use crate::usecase::workflow::startup::WorkflowStartupGateway;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

struct Trees {
    list_failures: AtomicUsize,
    block_list: bool,
    block_tree: Option<&'static str>,
    reconciled: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowStartupRepository for Trees {
    async fn list_tree_ids(&self) -> Result<Vec<String>, WorkflowError> {
        if self
            .list_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                (left > 0).then(|| left - 1)
            })
            == Ok(1)
            || self.list_failures.load(Ordering::SeqCst) > 0
        {
            return Err(WorkflowError::from(
                crate::domain::local_event::CommitBatchError::QueueBusy,
            ));
        }
        if self.block_list {
            std::future::pending::<()>().await;
        }
        Ok(["first", "second"].map(String::from).into())
    }

    async fn load(&self, _: &str) -> Result<Option<WorkflowStartupRecord>, WorkflowError> {
        Ok(None)
    }
}

#[async_trait::async_trait]
impl WorkflowStartupGateway for Trees {
    fn current_timestamp(&self) -> f64 {
        1.0
    }

    async fn reconcile_tree(&self, tree_id: &str, _: f64) -> Result<(), WorkflowError> {
        if self.block_tree == Some(tree_id) {
            std::future::pending::<()>().await;
        }
        self.reconciled.lock().unwrap().push(tree_id.into());
        Ok(())
    }
}

fn trees(list_failures: usize, block_list: bool, block_tree: Option<&'static str>) -> Arc<Trees> {
    Arc::new(Trees {
        list_failures: AtomicUsize::new(list_failures),
        block_list,
        block_tree,
        reconciled: Default::default(),
    })
}

#[tokio::test(start_paused = true)]
async fn test_起動時の再開_実行木ごとの試行に期限を掛け他の実行木を止めない() {
    // Given
    let trees = trees(0, false, Some("first"));
    let usecase = WorkflowStartupUsecase::new(trees.clone(), trees.clone());
    let (retrying, store) = crate::test_support::retry::test_retrying_with_store();
    let started = tokio::time::Instant::now();
    // When
    let error = recover(&retrying, &usecase).await.unwrap_err();
    // Then
    let WorkflowError::Store(failure) = error else {
        panic!("expected storage failure")
    };
    assert_eq!(failure.nature, TechnicalFailureNature::TimedOut);
    assert_eq!(started.elapsed(), ATTEMPT_LIMIT);
    assert_eq!(*trees.reconciled.lock().unwrap(), ["second"]);
    let records = store.records("first");
    assert_eq!(records.len(), 1);
    assert!(records[0].requires_attention);
    assert!(store.records("second").is_empty());
}

#[tokio::test(start_paused = true)]
async fn test_起動時の再開_一覧の一時的な失敗をやり直し期限切れを直前の失敗で上書きしない() {
    // Given
    let trees = trees(1, true, None);
    let usecase = WorkflowStartupUsecase::new(trees.clone(), trees.clone());
    let (retrying, store) = crate::test_support::retry::test_retrying_with_store();
    // When
    let error = recover(&retrying, &usecase).await.unwrap_err();
    // Then
    let WorkflowError::Store(failure) = error else {
        panic!("expected storage failure")
    };
    assert_eq!(failure.nature, TechnicalFailureNature::TimedOut);
    assert!(matches!(
        failure.source,
        StorageFailureSource::Technical(error) if error.message == attempt_expired().message
    ));
    assert!(store.records("daemon").is_empty());
    assert!(trees.reconciled.lock().unwrap().is_empty());
}
