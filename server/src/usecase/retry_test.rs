use super::*;
use crate::domain::failure::TechnicalFailureNature;
use crate::usecase::failure::WorkFailure;
use crate::usecase::failure::{BusinessFailure, Failure};
use crate::usecase::test_helpers::retry::test_retrying_with_store;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test(start_paused = true)]
async fn test_再試行記録_記録なしでも再試行し恒久失敗を返す() {
    for stage in [false, true] {
        let (retrying, store) = test_retrying_with_store();
        let calls = AtomicUsize::new(0);
        let operation = |_| {
            let transient = calls.fetch_add(1, Ordering::SeqCst) < 3;
            std::future::ready(Err::<(), _>(WorkFailure {
                kind: Failure::Technical(if transient {
                    TechnicalFailureNature::Transient
                } else {
                    TechnicalFailureNature::Other
                }),
                message: "failed".into(),
            }))
        };
        let result = if stage {
            retrying.stage(None, RetryBackoff::ITEM, operation).await
        } else {
            retrying.restart(None, RetryBackoff::ITEM, operation).await
        };
        assert_eq!(
            result.unwrap_err().kind,
            Failure::Technical(TechnicalFailureNature::Other)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert!(store.records("*").is_empty());
    }
}

#[tokio::test(start_paused = true)]
async fn test_やり直しの手順_分類による再試行と失敗の記録を同じ経路で行う() {
    for kind in [
        Failure::Technical(TechnicalFailureNature::Transient),
        Failure::Business(BusinessFailure::VersionConflict),
        Failure::Technical(TechnicalFailureNature::Other),
        Failure::Technical(TechnicalFailureNature::Cancelled),
        Failure::Business(BusinessFailure::Other),
    ] {
        // Given
        let (retrying, store) = test_retrying_with_store();
        let calls = AtomicUsize::new(0);
        let retryable = next_attempt(kind).is_some();
        // When
        let result = retrying
            .restart(
                FailureKey::new("repository_scan", "/repo"),
                RetryBackoff::ITEM,
                |progress| {
                    let call = calls.fetch_add(1, Ordering::SeqCst);
                    async move {
                        if call > 0 {
                            assert_eq!(
                                progress,
                                if kind == Failure::Technical(TechnicalFailureNature::Transient) {
                                    AttemptProgress::Continue
                                } else {
                                    AttemptProgress::Reload
                                }
                            );
                        }
                        if call < 6 {
                            Err(WorkFailure {
                                kind,
                                message: format!("failure {call}"),
                            })
                        } else {
                            Ok(())
                        }
                    }
                },
            )
            .await;
        let records = store.records("/repo");
        let attempts = calls.load(Ordering::SeqCst);
        // Then
        assert_eq!(result.is_ok(), retryable);
        assert_eq!(attempts, if retryable { 7 } else { 1 });
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].count, if retryable { 6 } else { 1 });
        assert_eq!(
            records[0].requires_attention,
            !retryable && kind != Failure::Technical(TechnicalFailureNature::Cancelled)
        );
    }
}

#[tokio::test(start_paused = true)]
async fn test_段階のやり直し_版の競合は記録せず呼び出し元へ返す() {
    // Given
    let (retrying, store) = test_retrying_with_store();
    let calls = AtomicUsize::new(0);
    // When
    let result = retrying
        .stage(
            FailureKey::new("workflow_control_plane", "exec"),
            RetryBackoff::CONFLICT,
            |_| {
                let call = calls.fetch_add(1, Ordering::SeqCst);
                async move {
                    Err::<(), _>(WorkFailure {
                        kind: if call == 0 {
                            Failure::Technical(TechnicalFailureNature::Transient)
                        } else {
                            Failure::Business(BusinessFailure::VersionConflict)
                        },
                        message: "failure".into(),
                    })
                }
            },
        )
        .await;
    let records = store.records("exec");
    // Then
    assert_eq!(
        result.unwrap_err().kind,
        Failure::Business(BusinessFailure::VersionConflict)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].kind,
        Failure::Technical(TechnicalFailureNature::Transient)
    );
}

#[tokio::test]
async fn test_試行の失敗記録_下位の再試行と上位への伝播を二重計上しない() {
    // Given
    let key = FailureKey::new("workflow_test", "retry-stage-count");
    let (retrying, store) = test_retrying_with_store();
    let calls = AtomicUsize::new(0);
    // When
    let result = retrying
        .restart(key.clone(), RetryBackoff::ITEM, |_| {
            retrying.stage(key.clone(), RetryBackoff::ITEM, |_| async {
                Err::<(), _>(WorkFailure {
                    kind: if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        Failure::Technical(TechnicalFailureNature::Transient)
                    } else {
                        Failure::Technical(TechnicalFailureNature::Other)
                    },
                    message: "failure".into(),
                })
            })
        })
        .await;
    let records = store.records(&key.target);
    // Then
    assert_eq!(
        result.unwrap_err().kind,
        Failure::Technical(TechnicalFailureNature::Other)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.count == 1));
}

#[tokio::test(start_paused = true)]
async fn test_やり直しの手順_成功で要対応を解消する() {
    // Given
    let (retrying, store) = test_retrying_with_store();
    let key = FailureKey::new("workflow_recovery", "tree");
    retrying.failures.observed(
        &key,
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: "repair".into(),
        },
    );
    let before = store.records("tree");
    // When
    retrying
        .restart(key, RetryBackoff::RECOVERY, |_| async {
            Ok::<_, WorkFailure>(())
        })
        .await
        .unwrap();
    let after = store.records("tree");
    // Then
    assert!(before[0].requires_attention);
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].count, 1);
    assert!(!after[0].requires_attention);
}
