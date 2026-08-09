use std::sync::Arc;

use super::{
    ProviderAgentSessionHistoryCandidateDto, ProviderAgentSessionHistoryPageDto,
    ProviderAgentSessionHistoryQueryError, ProviderAgentSessionHistoryQueryService,
    ProviderAgentSessionHistoryReadUsecase, ProviderAgentSessionHistoryRequest,
    ProviderAgentSessionProviderDto,
};

struct FixedHistoryQueryService;

#[async_trait::async_trait]
impl ProviderAgentSessionHistoryQueryService for FixedHistoryQueryService {
    async fn list(
        &self,
        _request: ProviderAgentSessionHistoryRequest,
    ) -> Result<ProviderAgentSessionHistoryPageDto, ProviderAgentSessionHistoryQueryError> {
        Ok(ProviderAgentSessionHistoryPageDto {
            items: vec![ProviderAgentSessionHistoryCandidateDto {
                provider: ProviderAgentSessionProviderDto::Claude,
                provider_session_id: "claude-1".to_string(),
                updated_at_ms: 10,
            }],
            next_after: None,
        })
    }
}

#[tokio::test]
async fn test_provider_agent_session_history_controller境界へusecaseとして公開する() {
    let usecase = ProviderAgentSessionHistoryReadUsecase::new(Arc::new(FixedHistoryQueryService));

    let page = usecase
        .list(ProviderAgentSessionHistoryRequest {
            worktree_path: "/repo/worktree".to_string(),
            limit: 1,
            after: None,
        })
        .await
        .unwrap();

    assert_eq!(page.items[0].provider_session_id, "claude-1");
}
