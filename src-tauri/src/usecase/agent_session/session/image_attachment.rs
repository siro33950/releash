#[cfg(test)]
use crate::domain::agent_session::services::detect_image_mime as domain_detect_image_mime;
use crate::domain::agent_session::services::{
    AttachmentExternalizationPolicy, DefaultAttachmentExternalizationPolicy,
};

pub fn validate_image_bytes(bytes: &[u8]) -> Result<&'static str, String> {
    DefaultAttachmentExternalizationPolicy.validate_image_bytes(bytes)
}

#[cfg(test)]
pub fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    domain_detect_image_mime(bytes)
}
