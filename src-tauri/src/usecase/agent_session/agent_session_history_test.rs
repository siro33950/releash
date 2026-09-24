use std::sync::Arc;

use super::{
    AgentSessionHistoryCandidateDto, AgentSessionHistoryPageDto, AgentSessionHistoryQueryError,
    AgentSessionHistoryQueryService, AgentSessionHistoryReadUsecase, AgentSessionHistoryRequest,
    AgentSessionProviderDto,
};

struct FixedHistoryQueryService;

#[async_trait::async_trait]
impl AgentSessionHistoryQueryService for FixedHistoryQueryService {
    async fn list(
        &self,
        _request: AgentSessionHistoryRequest,
    ) -> Result<AgentSessionHistoryPageDto, AgentSessionHistoryQueryError> {
        Ok(AgentSessionHistoryPageDto {
            items: vec![AgentSessionHistoryCandidateDto {
                provider: AgentSessionProviderDto::Claude,
                provider_session_id: "claude-1".to_string(),
                label: "Claude claude-1…".to_string(),
                updated_at_ms: 10,
            }],
            next_after: None,
        })
    }
}

#[tokio::test]
async fn test_agent_session_history_controller境界へusecaseとして公開する() {
    let usecase = AgentSessionHistoryReadUsecase::new(Arc::new(FixedHistoryQueryService));

    let page = usecase
        .list(AgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            limit: 1,
            after: None,
        })
        .await
        .unwrap();

    assert_eq!(page.items[0].provider_session_id, "claude-1");
}

#[test]
fn test_失敗分類_全変種と委譲した理由を保持する() {
    use crate::domain::failure::{ClassifiedFailure, FailureKind as F};
    use crate::usecase::agent_session::AgentSessionHistoryQueryError;
    // Given
    let cases = [
        (
            AgentSessionHistoryQueryError::InvalidRequest,
            F::InvalidInput,
        ),
        (AgentSessionHistoryQueryError::Unavailable, F::Temporary),
        (AgentSessionHistoryQueryError::Corrupt, F::Corrupt),
        (
            AgentSessionHistoryQueryError::Store(F::Temporary),
            F::Temporary,
        ),
        (
            AgentSessionHistoryQueryError::Store(F::RestartRequired),
            F::RestartRequired,
        ),
        (
            AgentSessionHistoryQueryError::Store(F::StateRequired),
            F::StateRequired,
        ),
        (
            AgentSessionHistoryQueryError::Store(F::InvalidInput),
            F::InvalidInput,
        ),
        (AgentSessionHistoryQueryError::Store(F::Expired), F::Expired),
        (AgentSessionHistoryQueryError::Store(F::Missing), F::Missing),
        (
            AgentSessionHistoryQueryError::Store(F::AlreadyPresent),
            F::AlreadyPresent,
        ),
        (
            AgentSessionHistoryQueryError::Store(F::Permission),
            F::Permission,
        ),
        (
            AgentSessionHistoryQueryError::Store(F::Capacity),
            F::Capacity,
        ),
        (
            AgentSessionHistoryQueryError::Store(F::Unsupported),
            F::Unsupported,
        ),
        (
            AgentSessionHistoryQueryError::Store(F::Internal),
            F::Internal,
        ),
        (AgentSessionHistoryQueryError::Store(F::Corrupt), F::Corrupt),
        (
            AgentSessionHistoryQueryError::Store(F::Cancelled),
            F::Cancelled,
        ),
        (AgentSessionHistoryQueryError::Store(F::Unknown), F::Unknown),
        (
            AgentSessionHistoryQueryError::Store(F::OutsideRange),
            F::OutsideRange,
        ),
        (
            AgentSessionHistoryQueryError::Store(F::AuthenticationRequired),
            F::AuthenticationRequired,
        ),
    ];
    for (error, expected) in cases {
        // When / Then
        assert_eq!(error.failure_kind(), expected, "{error:?}");
    }
}
