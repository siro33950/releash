use super::UsecaseError;
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceGatewayError, TerminalSurfaceInputUnavailableCause,
};

#[test]
fn test_入力失敗_失効attachmentを区別する() {
    // Given
    let error = TerminalSurfaceGatewayError::InputUnavailable(
        TerminalSurfaceInputUnavailableCause::StaleAttachment,
    );
    // When
    let result = UsecaseError::from(error);
    // Then
    assert_eq!(result, UsecaseError::StaleAttachment);
}

#[test]
fn test_terminal失敗_業務結果と全ての技術的性質を保持する() {
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        // Given
        let failure = TechnicalFailure {
            nature,
            message: "checkpoint failure".into(),
        };
        // When
        let error = UsecaseError::from(TerminalSurfaceGatewayError::Technical(failure.clone()));
        // Then
        assert_eq!(error, UsecaseError::Technical(failure));
    }
    assert_eq!(
        UsecaseError::from(TerminalSurfaceGatewayError::NotFound("missing".into())),
        UsecaseError::NotFound("missing".into())
    );
    assert_eq!(
        UsecaseError::from(TerminalSurfaceGatewayError::InvalidOperation(
            "exited".into()
        )),
        UsecaseError::InvalidOperation("exited".into())
    );
}

#[test]
fn test_terminal失敗_全変種から技術的な失敗だけを参照する() {
    use super::UsecaseError as E;
    use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
    // Given / When / Then
    for error in [
        E::NotFound("missing".into()),
        E::InvalidOperation("invalid".into()),
        E::OwnerConflict,
        E::StaleAttachment,
    ] {
        assert_eq!(error.technical_failure(), None);
    }
    for nature in [
        TechnicalFailureNature::Transient,
        TechnicalFailureNature::TimedOut,
        TechnicalFailureNature::Cancelled,
        TechnicalFailureNature::Other,
    ] {
        let failure = TechnicalFailure {
            nature,
            message: "source failure".into(),
        };
        assert_eq!(
            E::Technical(failure.clone()).technical_failure(),
            Some(&failure)
        );
    }
}
