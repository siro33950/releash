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
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
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

mod recovery_scenarios {
    use super::*;
    use crate::usecase::workflow::test_helpers::startup::*;
    use std::sync::Mutex;
    async fn execute(usecase: &WorkflowStartupUsecase) -> Result<(), WorkflowError> {
        super::recover(&crate::usecase::retry::test_retrying(), usecase).await
    }
    #[tokio::test]
    async fn test_起動時復旧_列挙と定義確認とreconciliationの順序を所有する() {
        // Given
        let startup = Arc::new(Startup {
            calls: Mutex::new(Vec::new()),
            failure: None,
            conflict: false,
            temporary: false,
        });
        let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());

        // When
        execute(&usecase).await.unwrap();

        // Then
        let pass = [
            "list",
            "load:first",
            "reconcile:first",
            "load:second",
            "reconcile:second",
        ];
        let calls = startup.calls.lock().unwrap();
        assert_eq!(calls.first().unwrap(), "list");
        for tree in ["first", "second"] {
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| call.ends_with(tree))
                    .cloned()
                    .collect::<Vec<_>>(),
                pass.iter()
                    .filter(|call| call.ends_with(tree))
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[tokio::test]
    async fn test_起動時復旧_定義確認に失敗したtreeを再生せず後続を処理して最初の失敗を返す() {
        for conflict in [false, true] {
            for failure in ["load:first", "reconcile:first"] {
                // Given
                let startup = Arc::new(Startup {
                    calls: Mutex::new(Vec::new()),
                    failure: Some(failure),
                    conflict,
                    temporary: false,
                });
                let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());

                // When
                let result = execute(&usecase).await;

                // Then
                if conflict {
                    result.unwrap();
                } else {
                    assert!(result.unwrap_err().to_string().contains(failure));
                }
                let calls = startup.calls.lock().unwrap();
                assert!(calls.contains(&"reconcile:second".into()));
                assert_eq!(
                    calls
                        .iter()
                        .filter(|call| call.as_str() == "reconcile:first")
                        .count(),
                    if conflict && failure == "reconcile:first" {
                        2
                    } else if conflict || failure == "reconcile:first" {
                        1
                    } else {
                        0
                    }
                );
            }
        }
    }

    #[tokio::test]
    async fn test_起動時復旧_列挙失敗は後続操作を呼ばず返す() {
        // Given
        let startup = Arc::new(Startup {
            calls: Mutex::new(Vec::new()),
            failure: Some("list"),
            conflict: false,
            temporary: false,
        });
        let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());
        // When / Then
        assert!(execute(&usecase)
            .await
            .unwrap_err()
            .to_string()
            .contains("list"));
        assert_eq!(*startup.calls.lock().unwrap(), ["list"]);
    }

    #[tokio::test]
    async fn test_起動時前進_競合後に読み直して再試行する() {
        for conflicts in [1, 5] {
            // Given
            let mut repository = repository();
            repository.unreadable = false;
            let repository = Arc::new(repository);
            let runtime = Arc::new(ConflictingStartup {
                conflicts,
                calls: Default::default(),
            });
            let usecase = WorkflowStartupUsecase::new(repository.clone(), runtime.clone());
            // When
            execute(&usecase).await.unwrap();
            // Then
            assert_eq!(
                runtime.calls.load(std::sync::atomic::Ordering::SeqCst),
                conflicts + 1
            );
        }
    }

    #[tokio::test]
    async fn test_起動時復旧_定義不明と前進失敗を要対応として記録する() {
        for unreadable in [false, true] {
            // Given
            let mut repository = repository();
            repository.unreadable = unreadable;
            let repository = Arc::new(repository);
            let runtime = Arc::new(FailedStartup::default());
            let usecase = WorkflowStartupUsecase::new(repository.clone(), runtime.clone());

            // When
            let (retrying, store) = crate::test_support::retry::test_retrying_with_store();
            let error = recover(&retrying, &usecase).await.unwrap_err();
            if unreadable {
                assert!(
                    matches!(error, WorkflowError::IncompatibleStoredEvent(reason) if reason.contains("completion"))
                );
            } else {
                assert!(
                    matches!(error, WorkflowError::External(message) if message == "advancement failed")
                );
            }

            // Then
            assert!(repository.terminal.lock().unwrap().is_none());
            assert_eq!(
                runtime.0.load(std::sync::atomic::Ordering::SeqCst),
                usize::from(!unreadable)
            );
            let observations = store.records("tree");
            assert_eq!(observations.len(), 1);
            assert!(observations[0].requires_attention);
        }
    }

    #[tokio::test]
    async fn test_起動時復旧_一覧の一時的失敗を再試行して各実行木を再開する() {
        let startup = Arc::new(Startup {
            calls: Mutex::new(Vec::new()),
            failure: Some("list"),
            conflict: false,
            temporary: true,
        });
        let usecase = WorkflowStartupUsecase::new(startup.clone(), startup.clone());
        execute(&usecase).await.unwrap();
        let calls = startup.calls.lock().unwrap();
        assert_eq!(&calls[..2], &["list", "list"]);
        for tree in ["first", "second"] {
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| **call == format!("reconcile:{tree}"))
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn test_起動復旧_作業列を通しても元のstore失敗を保持する() {
        use crate::domain::local_event::LocalEventQueryError;
        for source in [
            LocalEventQueryError::Corrupt {
                correlation_id: "corrupt".into(),
            },
            LocalEventQueryError::ResponseTooLarge,
            LocalEventQueryError::CanonicalWriterRequired,
            LocalEventQueryError::InvalidRequest,
        ] {
            // Given
            let source = crate::domain::failure::StorageFailure::from(source);
            let startup = WorkflowStartupUsecase::new(
                Arc::new(FailingStartupList(WorkflowError::Store(source.clone()))),
                Arc::new(Startup {
                    calls: Default::default(),
                    failure: None,
                    conflict: false,
                    temporary: false,
                }),
            );
            // When
            let error = execute(&startup).await.unwrap_err();
            // Then
            assert!(
                matches!(error, WorkflowError::Store(failure) if failure.source == source.source)
            );
        }
    }
}
