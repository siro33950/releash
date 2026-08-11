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
