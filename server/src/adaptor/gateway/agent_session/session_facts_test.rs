use super::*;
use crate::adaptor::presenter::connect::ConnectFailure;
use connectrpc::ErrorCode;

#[test]
fn test_session読取_混雑と期限切れをデータ破損扱いしない() {
    // Given
    for (error, expected) in [
        (LocalEventQueryError::QueryBusy, ErrorCode::Unavailable),
        (
            LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into(),
            }),
            ErrorCode::DeadlineExceeded,
        ),
    ] {
        // When / Then
        assert_eq!(
            AgentSessionRepositoryError::from(SessionContextReadError::Read(error.clone()))
                .connect_code(),
            expected
        );
        assert_eq!(
            AgentSessionQueryError::from(SessionContextReadError::Read(error)).connect_code(),
            expected
        );
    }
    let corrupt = LocalEventQueryError::Corrupt {
        correlation_id: "corrupt".into(),
    };
    assert_eq!(
        AgentSessionRepositoryError::from(SessionContextReadError::Read(corrupt.clone())),
        AgentSessionRepositoryError::Corrupt
    );
    assert_eq!(
        AgentSessionQueryError::from(SessionContextReadError::Read(corrupt)),
        AgentSessionQueryError::Corrupt
    );
}

#[test]
fn test_session読取_実効cwdの一時障害と破損をrepositoryとqueryへ区別して返す() {
    use crate::adaptor::gateway::workflow::worktree_context::WorktreeContextReadError;
    // Given
    for (error, expected) in [
        (LocalEventQueryError::QueryBusy, ErrorCode::Unavailable),
        (
            LocalEventQueryError::Technical(crate::domain::failure::TechnicalFailure {
                nature: crate::domain::failure::TechnicalFailureNature::TimedOut,
                message: "deadline exceeded".into(),
            }),
            ErrorCode::DeadlineExceeded,
        ),
        (
            crate::adaptor::gateway::local_event_store::reader::storage_unavailable(
                &rusqlite::Error::InvalidQuery,
            ),
            ErrorCode::Internal,
        ),
    ] {
        // When / Then
        assert_eq!(
            AgentSessionRepositoryError::from(SessionContextReadError::from(
                WorktreeContextReadError::Read(error.clone())
            ))
            .connect_code(),
            expected
        );
        assert_eq!(
            AgentSessionQueryError::from(SessionContextReadError::from(
                WorktreeContextReadError::Read(error)
            ))
            .connect_code(),
            expected
        );
    }
    for reason in [
        "parent execution is missing",
        "invalid worktree execution ancestry",
        "invalid worktree definition",
    ] {
        assert_eq!(
            AgentSessionRepositoryError::from(SessionContextReadError::from(
                WorktreeContextReadError::Corrupt(reason.into())
            )),
            AgentSessionRepositoryError::Corrupt
        );
        assert_eq!(
            AgentSessionQueryError::from(SessionContextReadError::from(
                WorktreeContextReadError::Corrupt(reason.into())
            )),
            AgentSessionQueryError::Corrupt
        );
    }
}
