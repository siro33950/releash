use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use crate::usecase::comment_image::{CommentImageDto, CommentImageQuery};

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub struct HttpCommentImageQuery {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl CommentImageQuery for HttpCommentImageQuery {
    async fn image(&self, url: &str) -> Result<CommentImageDto, TechnicalFailure> {
        crate::common::operation_context::timeout(
            std::time::Duration::from_secs(15),
            self.fetch(url),
        )
        .await
        .map_err(TechnicalFailure::from)?
    }
}

impl HttpCommentImageQuery {
    async fn fetch(&self, url: &str) -> Result<CommentImageDto, TechnicalFailure> {
        let url = reqwest::Url::parse(url).map_err(failure)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(failure(
                "Comment images require an HTTP(S) URL without credentials",
            ));
        }
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(failure)?
            .error_for_status()
            .map_err(failure)?;
        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !matches!(
            media_type.as_str(),
            "image/png"
                | "image/jpeg"
                | "image/gif"
                | "image/webp"
                | "image/avif"
                | "image/bmp"
                | "image/tiff"
        ) {
            return Err(failure("Unsupported Comment image content type"));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
        {
            return Err(failure("Comment image exceeds 8 MiB"));
        }
        let mut data = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(failure)? {
            if chunk.len() > MAX_IMAGE_BYTES - data.len() {
                return Err(failure("Comment image exceeds 8 MiB"));
            }
            data.extend_from_slice(&chunk);
        }
        Ok(CommentImageDto { data, media_type })
    }
}

fn failure(error: impl std::fmt::Display) -> TechnicalFailure {
    TechnicalFailure {
        nature: TechnicalFailureNature::Other,
        message: error.to_string(),
    }
}

#[cfg(test)]
#[path = "comment_image_test.rs"]
mod comment_image_tests;
