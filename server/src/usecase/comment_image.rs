use crate::domain::failure::TechnicalFailure;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentImageDto {
    pub data: Vec<u8>,
    pub media_type: String,
}

#[async_trait::async_trait]
pub trait CommentImageQuery: Send + Sync {
    async fn image(&self, url: &str) -> Result<CommentImageDto, TechnicalFailure>;
}
