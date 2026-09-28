use crate::domain::failure::TechnicalFailure;

#[async_trait::async_trait]
pub trait ReviewCommentsWatch: Send + Sync {
    async fn ensure_started(&self) -> Result<(), TechnicalFailure>;
    async fn poll(&self) -> Result<(), TechnicalFailure>;
    async fn restart(&self) -> Result<(), TechnicalFailure>;
}
