use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::usecase::agent_session::session::{
    ChatMessage, SessionMeta, SESSION_BODY_FORMAT_VERSION,
};

pub(super) fn sessions_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("sessions")
}

pub(super) fn session_titles_file(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("session_titles.json")
}

pub(super) static UUID_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
        .unwrap()
});

pub(super) fn session_file(app_data_dir: &Path, session_id: &str) -> Result<PathBuf, String> {
    if !UUID_RE.is_match(session_id) {
        return Err(format!("Invalid session_id: {session_id}"));
    }
    Ok(sessions_dir(app_data_dir).join(format!("{session_id}.json")))
}

pub(super) fn legacy_meta_file(app_data_dir: &Path, session_id: &str) -> Result<PathBuf, String> {
    if !UUID_RE.is_match(session_id) {
        return Err(format!("Invalid session_id: {session_id}"));
    }
    Ok(sessions_dir(app_data_dir).join(format!("{session_id}.meta.json")))
}

pub(super) fn session_dir(app_data_dir: &Path, session_id: &str) -> Result<PathBuf, String> {
    if !UUID_RE.is_match(session_id) {
        return Err(format!("Invalid session_id: {session_id}"));
    }
    Ok(sessions_dir(app_data_dir).join(session_id))
}

pub(super) fn meta_file_in_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("meta.json")
}

pub(super) fn index_file_in_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("index.json")
}

pub(super) fn private_context_file_in_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("private_context.json")
}

pub(super) fn event_log_file_in_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("events.json")
}

pub(super) fn messages_dir_in_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("messages")
}

pub(super) fn attachments_dir_in_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("attachments")
}

pub(super) fn tool_outputs_dir_in_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("tool_outputs")
}

pub(super) fn message_file_in_dir(session_dir: &Path, seq: u64) -> PathBuf {
    messages_dir_in_dir(session_dir).join(format!("{seq}.json"))
}

pub(super) fn attachment_file_in_dir(session_dir: &Path, attachment_id: &str) -> PathBuf {
    attachments_dir_in_dir(session_dir).join(attachment_id)
}

pub(super) fn tool_output_file_in_dir(session_dir: &Path, tool_output_id: &str) -> PathBuf {
    tool_outputs_dir_in_dir(session_dir).join(tool_output_id)
}

pub(super) fn invalid_session_error_message_with_id(session_id: &str) -> String {
    format!(
        "Invalid session data (id={session_id}, allowed permission modes: {})",
        crate::permission::PermissionMode::allowed_list()
    )
}

pub(super) fn invalid_session_error_message() -> String {
    format!(
        "Invalid session data (allowed permission modes: {})",
        crate::permission::PermissionMode::allowed_list()
    )
}

pub(super) fn content_hash(message: &ChatMessage) -> Result<String, String> {
    let bytes = serde_json::to_vec(message)
        .map_err(|e| format!("Failed to serialize message for hashing: {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(hex::encode(hasher.finalize()))
}

pub(super) fn write_json_pretty_atomic<T: Serialize>(
    path: &Path,
    value: &T,
    label: &str,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {label} dir: {e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    let write_result = (|| -> Result<(), String> {
        let file = std::fs::File::create(&tmp)
            .map_err(|e| format!("Failed to write {label} temp file: {e}"))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, value)
            .map_err(|e| format!("Failed to serialize {label}: {e}"))?;
        writer
            .flush()
            .map_err(|e| format!("Failed to flush {label} temp file: {e}"))?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("Failed to rename {label} temp file: {e}"))
}

pub(super) fn write_binary_atomic(path: &Path, bytes: &[u8], label: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {label} dir: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("Failed to write {label} temp file: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Failed to rename {label} temp file: {e}"))
}

pub(super) fn validate_permission_mode(permission_mode: &str) -> Result<String, String> {
    crate::permission::PermissionMode::parse(permission_mode)
        .map(|mode| mode.as_str().to_string())
        .map_err(|_| invalid_session_error_message())
}

pub(super) fn validate_meta(
    mut meta: SessionMeta,
    expected_id: &str,
) -> Result<SessionMeta, String> {
    if meta.id != expected_id {
        return Err(invalid_session_error_message_with_id(expected_id));
    }
    meta.permission_mode = validate_permission_mode(&meta.permission_mode)
        .map_err(|_| invalid_session_error_message_with_id(expected_id))?;
    if meta.body_format_version == 0 {
        meta.body_format_version = SESSION_BODY_FORMAT_VERSION;
    }
    Ok(meta)
}
