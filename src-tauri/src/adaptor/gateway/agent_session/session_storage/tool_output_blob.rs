use sha2::{Digest, Sha256};
use std::path::Path;

use super::layout::{
    content_hash, message_file_in_dir, session_dir, tool_output_file_in_dir,
    tool_outputs_dir_in_dir, write_binary_atomic,
};
use super::FileSessionStorage;
use crate::domain::agent_session::services::{
    DefaultToolOutputExternalizationPolicy, ToolOutputExternalizationPolicy,
};
use crate::usecase::agent_session::session::{
    parts_to_legacy, ChatMessage, MessagePart, SessionToolOutput, ToolOutputRef, ToolOutputSummary,
};

fn tool_output_id(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

pub(super) fn is_valid_tool_output_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_tool_output_ref(content_ref: &ToolOutputRef) -> Result<(), String> {
    if is_valid_tool_output_id(&content_ref.id) {
        Ok(())
    } else {
        Err(format!("Invalid tool output id: {}", content_ref.id))
    }
}

pub(super) fn tool_output_write_failure_log_message(
    message_id: &str,
    byte_size: usize,
    line_count: u64,
    err: impl std::fmt::Display,
) -> String {
    format!(
        "Failed to externalize tool output; keeping inline fallback: \
         message_id={message_id} byte_size={byte_size} line_count={line_count} error={err}"
    )
}

impl FileSessionStorage {
    pub fn get_session_tool_output(
        &self,
        app_data_dir: &Path,
        session_id: &str,
        tool_output_id: &str,
    ) -> Result<Option<SessionToolOutput>, String> {
        if !is_valid_tool_output_id(tool_output_id) {
            return Err(format!("Invalid tool output id: {tool_output_id}"));
        }
        self.ensure_loaded(app_data_dir)?;
        if let Some(err) = self.invalid_sessions.read().get(session_id) {
            return Err(err.clone());
        }
        if !self.cache.read().contains_key(session_id) {
            return Ok(None);
        }
        let dir = session_dir(app_data_dir, session_id)?;
        let mut index = self.read_consistent_index_from_dir(&dir, session_id)?;
        let referencing_entries = index
            .iter()
            .filter(|entry| {
                entry
                    .tool_output_refs
                    .iter()
                    .any(|content_ref| content_ref.id == tool_output_id)
            })
            .collect::<Vec<_>>();
        if referencing_entries.is_empty() {
            return Ok(None);
        }
        let mut has_matching_entry = false;
        for entry in referencing_entries {
            let message = match self.read_message_file(&message_file_in_dir(&dir, entry.seq)) {
                Ok(message) => message,
                Err(_) => continue,
            };
            if content_hash(&message)? == entry.content_hash {
                has_matching_entry = true;
                break;
            }
        }
        if !has_matching_entry {
            let _lock = self.file_lock.lock();
            index = self.repair_index_and_meta_from_messages(&dir, session_id)?;
            if !index
                .iter()
                .flat_map(|entry| entry.tool_output_refs.iter())
                .any(|content_ref| content_ref.id == tool_output_id)
            {
                return Ok(None);
            }
        }
        self.read_tool_output(&dir, tool_output_id)
    }

    pub(super) fn externalize_message_tool_outputs(
        &self,
        dir: &Path,
        message: ChatMessage,
    ) -> Result<ChatMessage, String> {
        let Some(parts) = message.parts.as_ref() else {
            return Ok(message);
        };
        let mut externalized_parts = Vec::with_capacity(parts.len());
        let mut has_tool_result = false;
        let policy = DefaultToolOutputExternalizationPolicy;
        for part in parts {
            match part {
                MessagePart::ToolResult {
                    content,
                    is_error,
                    tool_use_id,
                    parent_tool_use_id,
                    content_ref: Some(content_ref),
                    summary,
                } => {
                    has_tool_result = true;
                    validate_tool_output_ref(content_ref)?;
                    externalized_parts.push(MessagePart::ToolResult {
                        content: content.clone(),
                        is_error: *is_error,
                        tool_use_id: tool_use_id.clone(),
                        parent_tool_use_id: parent_tool_use_id.clone(),
                        content_ref: Some(content_ref.clone()),
                        summary: summary.clone(),
                    });
                }
                MessagePart::ToolResult {
                    content,
                    is_error,
                    tool_use_id,
                    parent_tool_use_id,
                    content_ref: None,
                    summary: _,
                } => {
                    has_tool_result = true;
                    if !policy.should_externalize_tool_output(content) {
                        externalized_parts.push(part.clone());
                        continue;
                    }
                    let id = tool_output_id(content);
                    let content_ref = ToolOutputRef {
                        id,
                        byte_size: content.len() as u64,
                    };
                    validate_tool_output_ref(&content_ref)?;
                    let path = tool_output_file_in_dir(dir, &content_ref.id);
                    let wrote_blob = if !path.exists() {
                        if let Err(err) =
                            write_binary_atomic(&path, content.as_bytes(), "tool output blob")
                        {
                            let summary = policy.tool_output_summary(content, *is_error, true);
                            let message = tool_output_write_failure_log_message(
                                &message.id,
                                content.len(),
                                summary.line_count,
                                err,
                            );
                            log::warn!("{message}");
                            externalized_parts.push(part.clone());
                            continue;
                        }
                        true
                    } else {
                        false
                    };
                    if wrote_blob {
                        crate::other::telemetry::record_tool_output_externalized(
                            content_ref.byte_size,
                        );
                    }
                    let projected_summary = policy.tool_output_summary(content, *is_error, true);
                    externalized_parts.push(MessagePart::ToolResult {
                        content: policy.tool_output_preview(content),
                        is_error: *is_error,
                        tool_use_id: tool_use_id.clone(),
                        parent_tool_use_id: parent_tool_use_id.clone(),
                        content_ref: Some(content_ref),
                        summary: Some(ToolOutputSummary {
                            line_count: projected_summary.line_count,
                            byte_size: projected_summary.byte_size,
                            is_error: projected_summary.is_error,
                            truncated: projected_summary.truncated,
                        }),
                    });
                }
                _ => externalized_parts.push(part.clone()),
            }
        }
        if !has_tool_result {
            return Ok(message);
        }
        let mut externalized = message;
        externalized.parts = Some(externalized_parts);
        if let Some(parts) = externalized.parts.as_deref() {
            let (_, _, activities) = parts_to_legacy(parts);
            externalized.activities = activities;
        }
        Ok(externalized)
    }

    pub(super) fn read_tool_output(
        &self,
        dir: &Path,
        tool_output_id: &str,
    ) -> Result<Option<SessionToolOutput>, String> {
        let tool_outputs_dir = tool_outputs_dir_in_dir(dir);
        let canonical_tool_outputs_dir = match tool_outputs_dir.canonicalize() {
            Ok(dir) => dir,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(format!("Failed to resolve tool outputs dir: {err}")),
        };
        let path = tool_output_file_in_dir(dir, tool_output_id);
        let canonical_path = match path.canonicalize() {
            Ok(path) => path,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(format!("Failed to resolve tool output blob: {err}")),
        };
        if !canonical_path.starts_with(&canonical_tool_outputs_dir) {
            return Err(format!(
                "Tool output path escaped tool outputs dir: {tool_output_id}"
            ));
        }
        let bytes = std::fs::read(canonical_path)
            .map_err(|e| format!("Failed to read tool output blob: {e}"))?;
        let content = String::from_utf8(bytes)
            .map_err(|e| format!("Failed to decode tool output blob: {e}"))?;
        Ok(Some(SessionToolOutput {
            byte_size: content.len() as u64,
            content,
        }))
    }

    pub(super) fn tool_output_refs_from_message(
        &self,
        message: &ChatMessage,
    ) -> Vec<ToolOutputRef> {
        let Some(parts) = message.parts.as_ref() else {
            return Vec::new();
        };
        parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::ToolResult {
                    content_ref: Some(content_ref),
                    ..
                } => Some(content_ref.clone()),
                _ => None,
            })
            .collect()
    }
}
