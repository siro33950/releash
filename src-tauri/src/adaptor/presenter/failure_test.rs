use super::*;
use crate::domain::failure::TechnicalFailureNature;
use crate::usecase::failure::{BusinessFailure, Failure};

fn presenter() -> (
    FailurePresenter,
    tokio::sync::broadcast::Receiver<StateChangeSource>,
) {
    let output = crate::test_support::state_subscription::test_output();
    let changes = crate::test_support::state_subscription::changes(&output);
    (
        FailurePresenter::new(Arc::new(FailureRecordStore::default()), Some(output)),
        changes,
    )
}

#[tokio::test]
async fn test_要対応の通知_設定と解除で同じ購読対象に通知する() {
    // Given
    let (presenter, mut changes) = presenter();
    let key = FailureKey::new("workflow_recovery", "tree");
    // When / Then
    presenter.observed(
        &key,
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: "repair".into(),
        },
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::WorkspaceList
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("tree".into())
    );
    presenter.observed(
        &key,
        WorkFailure {
            kind: Failure::Business(BusinessFailure::Other),
            message: "new repair reason".into(),
        },
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::WorkspaceList
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("tree".into())
    );
    assert_eq!(
        presenter.records("tree")[0].record.message,
        "new repair reason"
    );
    presenter.resolved(&key);
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::WorkspaceList
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("tree".into())
    );
}

#[tokio::test]
async fn test_要対応の通知_workflow以外の操作と要対応でない失敗は対象の購読だけに通知する() {
    // Given
    let (presenter, mut changes) = presenter();
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
    // Then
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("/repo".into())
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("tree".into())
    );
    assert_eq!(
        changes.recv().await.unwrap(),
        StateChangeSource::Failures("tree".into())
    );
    assert!(changes.try_recv().is_err());
}
