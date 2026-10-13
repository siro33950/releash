use super::*;
use crate::usecase::provider_usage::{
    ProviderUsage, ProviderUsageOutput, ProviderUsageQueryService, ProviderUsageSource,
    ProviderUsageUsecase, UsageError,
};
use std::sync::Arc;

struct PendingSource;
#[async_trait::async_trait]
impl ProviderUsageSource for PendingSource {
    fn claude(&self, _: &[u8]) -> Result<Option<ProviderUsage>, UsageError> {
        unreachable!()
    }
    async fn codex(&self, _: &str) -> Result<Option<(String, ProviderUsage)>, UsageError> {
        std::future::pending().await
    }
}
struct Projection;
impl ProviderUsageQueryService for Projection {
    fn snapshot(&self) -> Vec<ProviderUsage> {
        Vec::new()
    }
}
impl ProviderUsageOutput for Projection {
    fn replace(&self, _: ProviderUsage) {
        unreachable!()
    }
}

#[tokio::test]
async fn test_使用率準備_読取りが要求期限を超えたら型付きの使用率失敗を返す() {
    // Given
    let usage = ProviderUsageUsecase::new(
        Arc::new(PendingSource),
        Arc::new(Projection),
        Arc::new(Projection),
        crate::test_support::state_subscription::test_subscriptions(),
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(30);
    // When
    let error = prepare_usage(
        Some(deadline),
        Box::pin(async move { usage.seed_codex_session("session").await }),
    )
    .await
    .unwrap_err();
    // Then
    assert!(
        matches!(error, UsageError::Technical(error) if error.nature == crate::domain::failure::TechnicalFailureNature::TimedOut)
    );
}
