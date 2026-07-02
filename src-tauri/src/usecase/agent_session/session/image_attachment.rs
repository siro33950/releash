use crate::domain::agent_session::services::{
    AttachmentExternalizationPolicy, DefaultAttachmentExternalizationPolicy,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAttachment {
    pub data: String,
    pub media_type: String,
}

pub fn validate_image_bytes(bytes: &[u8]) -> Result<&'static str, String> {
    DefaultAttachmentExternalizationPolicy.validate_image_bytes(bytes)
}

pub fn prepare_image_attachment_data(data: Vec<u8>) -> Result<ImageAttachment, String> {
    if data.is_empty() {
        return Err("Empty image data".to_string());
    }
    validate_and_encode_image(&data)
}

pub async fn prepare_image_attachments_from_paths_usecase(
    paths: Vec<String>,
) -> Result<Vec<ImageAttachment>, String> {
    let mut attachments = Vec::new();
    for path in &paths {
        let data = tokio::fs::read(path)
            .await
            .map_err(|error| format!("Failed to read {path}: {error}"))?;
        if data.is_empty() {
            continue;
        }
        if let Ok(attachment) = validate_and_encode_image(&data) {
            attachments.push(attachment);
        }
    }
    Ok(attachments)
}

fn validate_and_encode_image(bytes: &[u8]) -> Result<ImageAttachment, String> {
    let media_type = validate_image_bytes(bytes)?;
    use base64::Engine;
    Ok(ImageAttachment {
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        media_type: media_type.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_image_attachment_data_validates_and_base64_encodes() {
        assert!(prepare_image_attachment_data(Vec::new()).is_err());

        let attachment = prepare_image_attachment_data(vec![0x89, 0x50, 0x4E, 0x47]).unwrap();

        assert_eq!(attachment.media_type, "image/png");
        assert_eq!(attachment.data, "iVBORw==");
    }
}
