use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSessionTitleRequest {
    pub provider: ProviderKind,
    pub provider_session_id: String,
    pub worktree_path: String,
    pub transcript_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSessionTitleGatewayError {
    Technical(crate::domain::failure::TechnicalFailure),
    Corrupt,
}

#[async_trait::async_trait]
pub trait ProviderSessionTitleGateway: Send + Sync {
    async fn read_title(
        &self,
        request: ProviderSessionTitleRequest,
    ) -> Result<Option<String>, ProviderSessionTitleGatewayError>;
}
