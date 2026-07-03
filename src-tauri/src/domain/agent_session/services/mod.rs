pub(crate) mod context_replacement;
pub(crate) mod skill_frontmatter;

use super::SkillEntry;

pub(crate) use context_replacement::{
    dedup_instructions, latest_revisions_by_kind, next_epoch_for_identity,
    normalize_path_components, replacement_action, snapshot_is_stale,
};
pub(crate) use skill_frontmatter::parse_skill_frontmatter;

/// Maximum image size in bytes (5 MiB).
/// Anthropic Messages API limits base64-encoded images to roughly 5 MB.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

pub const MAX_TOOL_OUTPUT_BYTES: usize = 30 * 1024;
pub const MAX_TOOL_OUTPUT_LINES: usize = 1000;
pub const TOOL_OUTPUT_PREVIEW_BYTES: usize = MAX_TOOL_OUTPUT_BYTES;
pub const TOOL_OUTPUT_PREVIEW_LINES: usize = MAX_TOOL_OUTPUT_LINES;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputSummaryProjection {
    pub line_count: u64,
    pub byte_size: u64,
    pub is_error: bool,
    pub truncated: bool,
}

pub trait ToolOutputExternalizationPolicy {
    fn should_externalize_tool_output(&self, content: &str) -> bool;
    fn tool_output_preview(&self, content: &str) -> String;
    fn tool_output_summary(
        &self,
        content: &str,
        is_error: bool,
        truncated: bool,
    ) -> ToolOutputSummaryProjection;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultToolOutputExternalizationPolicy;

impl ToolOutputExternalizationPolicy for DefaultToolOutputExternalizationPolicy {
    fn should_externalize_tool_output(&self, content: &str) -> bool {
        tool_output_line_count(content) > MAX_TOOL_OUTPUT_LINES as u64
            || content.len() > MAX_TOOL_OUTPUT_BYTES
    }

    fn tool_output_preview(&self, content: &str) -> String {
        let line_limited = if tool_output_line_count(content) <= TOOL_OUTPUT_PREVIEW_LINES as u64 {
            content
        } else {
            let mut end = 0;
            for (line_index, chunk) in content.split_inclusive('\n').enumerate() {
                if line_index >= TOOL_OUTPUT_PREVIEW_LINES {
                    break;
                }
                end += chunk.len();
            }
            &content[..end]
        };
        truncate_to_char_boundary(line_limited, TOOL_OUTPUT_PREVIEW_BYTES).to_string()
    }

    fn tool_output_summary(
        &self,
        content: &str,
        is_error: bool,
        truncated: bool,
    ) -> ToolOutputSummaryProjection {
        ToolOutputSummaryProjection {
            line_count: tool_output_line_count(content),
            byte_size: content.len() as u64,
            is_error,
            truncated,
        }
    }
}

pub fn tool_output_line_count(content: &str) -> u64 {
    content.lines().count() as u64
}

fn truncate_to_char_boundary(content: &str, max_bytes: usize) -> &str {
    if content.len() <= max_bytes {
        return content;
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    &content[..end]
}

pub trait AttachmentExternalizationPolicy {
    fn reject_oversized_base64_image(&self, data: &str) -> Result<(), String>;
    fn validate_image_bytes(&self, bytes: &[u8]) -> Result<&'static str, String>;
    fn validate_image_bytes_for_media_type(
        &self,
        bytes: &[u8],
        media_type: &str,
    ) -> Result<&'static str, String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultAttachmentExternalizationPolicy;

impl AttachmentExternalizationPolicy for DefaultAttachmentExternalizationPolicy {
    fn reject_oversized_base64_image(&self, data: &str) -> Result<(), String> {
        let max_encoded_len = max_base64_image_len();
        if data.len() > max_encoded_len {
            return Err(format!(
                "Image too large: encoded length {} exceeds max encoded length {}",
                data.len(),
                max_encoded_len
            ));
        }
        Ok(())
    }

    fn validate_image_bytes(&self, bytes: &[u8]) -> Result<&'static str, String> {
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(format!(
                "Image too large: {} bytes (max {} bytes)",
                bytes.len(),
                MAX_IMAGE_BYTES
            ));
        }

        detect_image_mime(bytes).ok_or_else(|| "Unsupported image format".to_string())
    }

    fn validate_image_bytes_for_media_type(
        &self,
        bytes: &[u8],
        media_type: &str,
    ) -> Result<&'static str, String> {
        let detected = self.validate_image_bytes(bytes)?;
        if detected != media_type {
            return Err(format!(
                "Image media type mismatch: declared {media_type}, detected {detected}"
            ));
        }
        Ok(detected)
    }
}

pub fn max_base64_image_len() -> usize {
    MAX_IMAGE_BYTES.div_ceil(3) * 4
}

/// Detect MIME type from magic bytes.
pub fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 4 {
        return None;
    }
    if bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
        return Some("image/jpeg");
    }
    if bytes[0] == 0x89 && bytes[1] == 0x50 && bytes[2] == 0x4E && bytes[3] == 0x47 {
        return Some("image/png");
    }
    if bytes[0] == 0x47 && bytes[1] == 0x49 && bytes[2] == 0x46 && bytes[3] == 0x38 {
        return Some("image/gif");
    }
    if bytes.len() >= 12
        && bytes[0] == 0x52
        && bytes[1] == 0x49
        && bytes[2] == 0x46
        && bytes[3] == 0x46
        && bytes[8] == 0x57
        && bytes[9] == 0x45
        && bytes[10] == 0x42
        && bytes[11] == 0x50
    {
        return Some("image/webp");
    }
    None
}

pub(crate) fn filter_agent_skills_for_query(
    skills: Vec<SkillEntry>,
    query: Option<&str>,
    limit: Option<usize>,
) -> Vec<SkillEntry> {
    let needle = query.unwrap_or_default().trim().to_lowercase();
    let max_results = limit.unwrap_or(usize::MAX);
    skills
        .into_iter()
        .filter(|skill| {
            needle.is_empty()
                || skill.name.to_lowercase().contains(&needle)
                || skill.description.to_lowercase().contains(&needle)
        })
        .take(max_results)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_agent_skills_filters_by_query_and_limit() {
        let skills = vec![
            SkillEntry {
                name: "review".to_string(),
                description: "Review code changes".to_string(),
                scope: "project".to_string(),
            },
            SkillEntry {
                name: "docs".to_string(),
                description: "Write documentation".to_string(),
                scope: "personal".to_string(),
            },
            SkillEntry {
                name: "diagram".to_string(),
                description: "Document architecture diagrams".to_string(),
                scope: "project".to_string(),
            },
        ];

        let result = filter_agent_skills_for_query(skills.clone(), Some("doc"), Some(1));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "docs");

        let result = filter_agent_skills_for_query(skills, Some("review"), Some(20));

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "review");
    }
}
