use super::*;
use crate::domain::failure::TechnicalFailureNature;
use crate::usecase::failure::{BusinessFailure, Failure};

#[test]
fn test_子プロセス失敗分類_全分類を変更せず往復する() {
    for kind in [
        Failure::Business(BusinessFailure::VersionConflict),
        Failure::Business(BusinessFailure::Other),
        Failure::Technical(TechnicalFailureNature::Transient),
        Failure::Technical(TechnicalFailureNature::TimedOut),
        Failure::Technical(TechnicalFailureNature::Cancelled),
        Failure::Technical(TechnicalFailureNature::Other),
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
