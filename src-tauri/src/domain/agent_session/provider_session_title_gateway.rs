use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderSessionTitleRequest {
    pub(crate) provider: ProviderKind,
    pub(crate) provider_session_id: String,
    pub(crate) worktree_path: String,
    pub(crate) transcript_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderSessionTitleGatewayError {
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait ProviderSessionTitleGateway: Send + Sync {
    async fn read_title(
        &self,
        request: ProviderSessionTitleRequest,
    ) -> Result<Option<String>, ProviderSessionTitleGatewayError>;
}

impl crate::domain::failure::ClassifiedFailure for ProviderSessionTitleGatewayError {
    fn failure_kind(&self) -> crate::domain::failure::FailureKind {
        match self {
            Self::Unavailable => crate::domain::failure::FailureKind::Temporary,
            Self::Corrupt => crate::domain::failure::FailureKind::Corrupt,
        }
    }
}
