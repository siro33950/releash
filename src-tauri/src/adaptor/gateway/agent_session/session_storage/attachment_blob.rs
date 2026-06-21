use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use sha2::{Digest, Sha256};
use std::path::Path;

use super::layout::{
    attachment_file_in_dir, attachments_dir_in_dir, session_dir, write_binary_atomic,
};
use super::FileSessionStorage;
use crate::usecase::agent_session::session::{
    AttachmentRef, ChatMessage, MessagePart, SessionAttachment,
};

fn attachment_id(media_type: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(media_type.as_bytes());
    hasher.update([0]);
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub(super) fn is_valid_attachment_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_attachment_ref(attachment: &AttachmentRef) -> Result<(), String> {
    if is_valid_attachment_id(&attachment.id) {
        Ok(())
    } else {
        Err(format!("Invalid attachment id: {}", attachment.id))
    }
}

impl FileSessionStorage {
    pub fn get_session_attachment(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<SessionAttachment>, String> {
        if !is_valid_attachment_id(attachment_id) {
            return Err(format!("Invalid attachment id: {attachment_id}"));
        }
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Ok(None);
        }
        self.ensure_session_layout(app_data_dir, session_id)?;
        let dir = session_dir(app_data_dir, session_id)?;
        let index = self.read_consistent_index_from_dir(&dir, session_id)?;
        let Some(attachment) = index
            .iter()
            .flat_map(|entry| entry.attachment_refs.iter())
            .find(|attachment| attachment.id == attachment_id)
            .cloned()
        else {
            return Ok(None);
        };
        self.hydrate_attachment(&dir, &attachment).map(Some)
    }
    pub(super) fn externalize_message_attachments(
        &self,
        dir: &Path,
        message: &ChatMessage,
    ) -> Result<(ChatMessage, Vec<AttachmentRef>), String> {
        let Some(parts) = message.parts.as_ref() else {
            return Ok((message.clone(), Vec::new()));
        };
        let mut refs = Vec::new();
        let mut externalized_parts = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                MessagePart::Image { data, media_type } => {
                    let bytes = BASE64_STANDARD
                        .decode(data)
                        .map_err(|e| format!("Failed to decode image attachment: {e}"))?;
                    let attachment = AttachmentRef {
                        id: attachment_id(media_type, &bytes),
                        media_type: media_type.clone(),
                        byte_size: bytes.len() as u64,
                    };
                    validate_attachment_ref(&attachment)?;
                    let path = attachment_file_in_dir(dir, &attachment.id);
                    if !path.exists() {
                        write_binary_atomic(&path, &bytes, "attachment blob")?;
                    }
                    refs.push(attachment.clone());
                    externalized_parts.push(MessagePart::ImageRef { attachment });
                }
                MessagePart::ImageRef { attachment } => {
                    validate_attachment_ref(attachment)?;
                    refs.push(attachment.clone());
                    externalized_parts.push(part.clone());
                }
                _ => externalized_parts.push(part.clone()),
            }
        }
        let mut externalized = message.clone();
        externalized.parts = Some(externalized_parts);
        Ok((externalized, refs))
    }

    pub(super) fn hydrate_message_attachments(
        &self,
        dir: &Path,
        message: ChatMessage,
    ) -> Result<ChatMessage, String> {
        let Some(parts) = message.parts.as_ref() else {
            return Ok(message);
        };
        if !parts
            .iter()
            .any(|part| matches!(part, MessagePart::ImageRef { .. }))
        {
            return Ok(message);
        }
        let mut hydrated_parts = Vec::with_capacity(parts.len());
        for part in parts {
            match part {
                MessagePart::ImageRef { attachment } => {
                    let image = self.hydrate_attachment(dir, attachment)?;
                    hydrated_parts.push(MessagePart::Image {
                        data: image.data,
                        media_type: image.media_type,
                    });
                }
                _ => hydrated_parts.push(part.clone()),
            }
        }
        let mut hydrated = message;
        hydrated.parts = Some(hydrated_parts);
        Ok(hydrated)
    }

    pub(super) fn hydrate_attachment(
        &self,
        dir: &Path,
        attachment: &AttachmentRef,
    ) -> Result<SessionAttachment, String> {
        validate_attachment_ref(attachment)?;
        let attachments_dir = attachments_dir_in_dir(dir);
        let canonical_attachments_dir = attachments_dir
            .canonicalize()
            .map_err(|e| format!("Failed to resolve attachments dir: {e}"))?;
        let path = attachment_file_in_dir(dir, &attachment.id);
        let canonical_path = path
            .canonicalize()
            .map_err(|e| format!("Failed to resolve attachment blob: {e}"))?;
        if !canonical_path.starts_with(&canonical_attachments_dir) {
            return Err(format!(
                "Attachment path escaped attachments dir: {}",
                attachment.id
            ));
        }
        let bytes = std::fs::read(canonical_path)
            .map_err(|e| format!("Failed to read attachment blob: {e}"))?;
        Ok(SessionAttachment {
            data: BASE64_STANDARD.encode(bytes),
            media_type: attachment.media_type.clone(),
        })
    }

    pub(super) fn attachment_refs_from_message(&self, message: &ChatMessage) -> Vec<AttachmentRef> {
        message
            .parts
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|part| match part {
                MessagePart::ImageRef { attachment } if is_valid_attachment_id(&attachment.id) => {
                    Some(attachment.clone())
                }
                _ => None,
            })
            .collect()
    }
}
