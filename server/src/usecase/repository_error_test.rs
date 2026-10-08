#[test]
fn test_worktree削除待ちの失敗_期限と取り消しの分類を維持する() {
    for nature in [
        crate::domain::failure::TechnicalFailureNature::TimedOut,
        crate::domain::failure::TechnicalFailureNature::Cancelled,
    ] {
        let error = super::UsecaseError::from(crate::domain::workflow::WorkflowError::Store(
            crate::domain::failure::TechnicalFailure {
                nature,
                message: "stopped".into(),
            }
            .into(),
        ));
        assert!(
            matches!(error, super::UsecaseError::Workflow(crate::domain::workflow::WorkflowError::Store(failure)) if failure.nature == nature && failure.source == crate::domain::failure::StorageFailureSource::Technical(crate::domain::failure::TechnicalFailure { nature, message: "stopped".into() }))
        );
    }
}
