use super::{CodexUsageRecord, ProviderUsage, UsageError};
#[async_trait::async_trait]
pub trait ProviderUsageSource: Send + Sync {
    fn claude(&self, payload: &[u8]) -> Result<Option<ProviderUsage>, UsageError>;
    async fn codex(&self, session: &str) -> Result<Option<CodexUsageRecord>, UsageError>;
}
