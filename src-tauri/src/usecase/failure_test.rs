use super::*;
use crate::adaptor::gateway::failure_records::FailureRecordStore;
use std::sync::Arc;

#[test]
fn test_失敗分類_六種類のdomain値を出力へ写す() {
    // Given
    let cases = [
        (
            Failure::Business(BusinessFailure::VersionConflict),
            FailureClassificationDto::VersionConflict,
        ),
        (
            Failure::Business(BusinessFailure::Other),
            FailureClassificationDto::BusinessFailure,
        ),
        (
            Failure::Technical(TechnicalFailureNature::Transient),
            FailureClassificationDto::Transient,
        ),
        (
            Failure::Technical(TechnicalFailureNature::TimedOut),
            FailureClassificationDto::TimedOut,
        ),
        (
            Failure::Technical(TechnicalFailureNature::Cancelled),
            FailureClassificationDto::Cancelled,
        ),
        (
            Failure::Technical(TechnicalFailureNature::Other),
            FailureClassificationDto::TechnicalFailure,
        ),
    ];

    // When
    for (domain, expected) in cases {
        let actual = FailureClassificationDto::from(domain);
        // Then
        assert_eq!(actual, expected);
    }
}

#[test]
fn test_要対応判定_六種類の失敗の意味だけから決まる() {
    for (kind, expected) in [
        (Failure::Business(BusinessFailure::VersionConflict), false),
        (Failure::Business(BusinessFailure::Other), true),
        (Failure::Technical(TechnicalFailureNature::Transient), false),
        (Failure::Technical(TechnicalFailureNature::TimedOut), true),
        (Failure::Technical(TechnicalFailureNature::Cancelled), false),
        (Failure::Technical(TechnicalFailureNature::Other), true),
    ] {
        assert_eq!(requires_attention(kind), expected);
    }
}

#[test]
fn test_やり直しの判断_版の競合は読み直し一時的な失敗は続行で他はやり直さない() {
    assert_eq!(
        next_attempt(Failure::Business(BusinessFailure::VersionConflict)),
        Some(AttemptProgress::Reload)
    );
    assert_eq!(
        next_attempt(Failure::Technical(TechnicalFailureNature::Transient)),
        Some(AttemptProgress::Continue)
    );
    for kind in [
        Failure::Business(BusinessFailure::Other),
        Failure::Technical(TechnicalFailureNature::TimedOut),
        Failure::Technical(TechnicalFailureNature::Cancelled),
        Failure::Technical(TechnicalFailureNature::Other),
    ] {
        assert_eq!(next_attempt(kind), None);
    }
}

#[test]
fn test_記録する失敗_作業の失敗は文面をそのまま他はdebug表記を使う() {
    let failure = WorkFailure {
        kind: Failure::Business(BusinessFailure::Other),
        message: "plain".into(),
    };
    assert_eq!(failure.work_failure(), failure);
    let error = crate::domain::workflow::WorkflowError::Conflict("stale".into());
    let recorded = error.work_failure();
    assert_eq!(
        recorded.kind,
        Failure::Business(BusinessFailure::VersionConflict)
    );
    assert_eq!(recorded.message, format!("{error:?}"));
}

#[test]
fn test_失敗記録_usecaseが要対応を判定してrepositoryへ渡す() {
    // Given
    let store = Arc::new(crate::adaptor::gateway::failure_records::FailureRecordStore::default());
    let recording = FailureRecordingUsecase::new(store.clone(), None);
    let key = FailureKey::new("workflow_start", "tree");
    // When
    recording.observed(
        &key,
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: "repair".into(),
        },
    );
    // Then
    assert!(store.records("tree")[0].record.requires_attention);
    // When
    recording.observed(
        &key,
        WorkFailure {
            kind: Failure::Technical(TechnicalFailureNature::Transient),
            message: "busy".into(),
        },
    );
    // Then
    assert!(
        !store
            .records("tree")
            .last()
            .unwrap()
            .record
            .requires_attention
    );
}

fn presenter() -> (
    FailureRecordingUsecase,
    Arc<FailureRecordStore>,
    tokio::sync::broadcast::Receiver<crate::usecase::state_subscription::StateChangeSource>,
) {
    let output = crate::test_support::state_subscription::test_subscriptions();
    let changes = crate::test_support::state_subscription::changes(&output);
    let store = Arc::new(FailureRecordStore::default());
    (
        FailureRecordingUsecase::new(store.clone(), Some(output)),
        store,
        changes,
    )
}

#[tokio::test]
async fn test_要対応の通知_設定と解除で同じ購読対象に通知する() {
    // Given
    let (presenter, store, mut changes) = presenter();
    let key = FailureKey::new("workflow_recovery", "tree");
    // When
    presenter.observed(
        &key,
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: "repair".into(),
        },
    );
    presenter.observed(
        &key,
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: "new repair reason".into(),
        },
    );
    let records = store.records("tree");
    presenter.resolved(&key);
    let received = [
        changes.recv().await.unwrap(),
        changes.recv().await.unwrap(),
        changes.recv().await.unwrap(),
        changes.recv().await.unwrap(),
        changes.recv().await.unwrap(),
        changes.recv().await.unwrap(),
    ];
    // Then
    assert_eq!(
        received,
        [
            crate::usecase::state_subscription::StateChangeSource::WorkspaceList,
            crate::usecase::state_subscription::StateChangeSource::Failures("tree".into()),
            crate::usecase::state_subscription::StateChangeSource::WorkspaceList,
            crate::usecase::state_subscription::StateChangeSource::Failures("tree".into()),
            crate::usecase::state_subscription::StateChangeSource::WorkspaceList,
            crate::usecase::state_subscription::StateChangeSource::Failures("tree".into()),
        ]
    );
    assert_eq!(records[0].record.message, "new repair reason");
}

#[tokio::test]
async fn test_要対応の通知_workflow以外の操作と要対応でない失敗は対象の購読だけに通知する() {
    // Given
    let (presenter, _, mut changes) = presenter();
    // When
    presenter.observed(
        &FailureKey::new("repository_scan", "/repo"),
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: "broken".into(),
        },
    );
    presenter.observed(
        &FailureKey::new("workflow_recovery", "tree"),
        WorkFailure {
            kind: Failure::Technical(TechnicalFailureNature::Transient),
            message: "busy".into(),
        },
    );
    presenter.resolved(&FailureKey::new("workflow_recovery", "tree"));
    let received = [
        changes.recv().await.unwrap(),
        changes.recv().await.unwrap(),
        changes.recv().await.unwrap(),
    ];
    let no_more_changes = changes.try_recv().is_err();
    // Then
    assert_eq!(
        received,
        [
            crate::usecase::state_subscription::StateChangeSource::Failures("/repo".into()),
            crate::usecase::state_subscription::StateChangeSource::Failures("tree".into()),
            crate::usecase::state_subscription::StateChangeSource::Failures("tree".into()),
        ]
    );
    assert!(no_more_changes);
}
