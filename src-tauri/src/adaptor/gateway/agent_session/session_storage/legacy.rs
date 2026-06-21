use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use std::io::BufReader;
use std::path::Path;

use super::layout::{
    file_timestamp_fallback, invalid_session_error_message, invalid_session_error_message_with_id,
    legacy_meta_file, meta_file_in_dir, validate_meta, validate_permission_mode,
    write_json_pretty_atomic,
};
use super::FileSessionStorage;
use crate::usecase::agent_session::session::{
    ChatSession, ContextCarryState, SessionMeta, SessionState, SESSION_BODY_FORMAT_VERSION,
};

#[derive(Default)]
struct FlatSessionMetaFile {
    id: Option<String>,
    worktree_path: Option<String>,
    state: Option<SessionState>,
    created_at: Option<f64>,
    updated_at: Option<f64>,
    agent_session_id: Option<String>,
    context_carry: Option<ContextCarryState>,
    permission_mode: Option<String>,
    plan_mode: bool,
    selected_model: Option<String>,
    permission_profile_id: Option<String>,
    backend_id: Option<String>,
    workflow_step_session: bool,
    first_message_preview: String,
    message_count: usize,
}

impl<'de> Deserialize<'de> for FlatSessionMetaFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(FlatSessionMetaVisitor)
    }
}

struct FlatSessionMetaVisitor;

impl<'de> Visitor<'de> for FlatSessionMetaVisitor {
    type Value = FlatSessionMetaFile;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a legacy flat chat session object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut meta = FlatSessionMetaFile::default();
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "id" => meta.id = map.next_value()?,
                "worktreePath" => meta.worktree_path = map.next_value()?,
                "state" => meta.state = map.next_value()?,
                "createdAt" => meta.created_at = map.next_value()?,
                "updatedAt" => meta.updated_at = map.next_value()?,
                "agentSessionId" => meta.agent_session_id = map.next_value()?,
                "contextCarry" => meta.context_carry = map.next_value()?,
                "permissionMode" => meta.permission_mode = map.next_value()?,
                "planMode" => meta.plan_mode = map.next_value()?,
                "selectedModel" => meta.selected_model = map.next_value()?,
                "permissionProfileId" => meta.permission_profile_id = map.next_value()?,
                "backendId" => meta.backend_id = map.next_value()?,
                "workflowStepSession" => meta.workflow_step_session = map.next_value()?,
                "messages" => {
                    let summary = map.next_value::<FlatMessagesSummary>()?;
                    meta.first_message_preview = summary.first_message_preview;
                    meta.message_count = summary.message_count;
                }
                _ => {
                    let _ = map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(meta)
    }
}

struct FlatMessagesSummary {
    first_message_preview: String,
    message_count: usize,
}

#[derive(Default)]
struct FlatMessagePreview {
    content: String,
    has_image_part: bool,
}

impl FlatMessagePreview {
    fn preview(&self) -> String {
        let content = if self.content.is_empty() && self.has_image_part {
            "[Image]".to_string()
        } else {
            self.content.clone()
        };
        match content.char_indices().nth(100) {
            Some((byte_pos, _)) => format!("{}…", &content[..byte_pos]),
            None => content,
        }
    }
}

impl<'de> Deserialize<'de> for FlatMessagePreview {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(FlatMessagePreviewVisitor)
    }
}

struct FlatMessagePreviewVisitor;

impl<'de> Visitor<'de> for FlatMessagePreviewVisitor {
    type Value = FlatMessagePreview;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a legacy flat chat message preview object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut preview = FlatMessagePreview::default();
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "content" => preview.content = map.next_value()?,
                "parts" => {
                    preview.has_image_part = map
                        .next_value::<Option<FlatMessagePartsSummary>>()?
                        .is_some_and(|parts| parts.has_image)
                }
                _ => {
                    let _ = map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(preview)
    }
}

struct FlatMessagePartsSummary {
    has_image: bool,
}

impl<'de> Deserialize<'de> for FlatMessagePartsSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(FlatMessagePartsSummaryVisitor)
    }
}

struct FlatMessagePartsSummaryVisitor;

impl<'de> Visitor<'de> for FlatMessagePartsSummaryVisitor {
    type Value = FlatMessagePartsSummary;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a legacy flat chat message parts array")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut has_image = false;
        while let Some(part) = seq.next_element::<FlatMessagePartSummary>()? {
            has_image |= part.has_image;
        }
        Ok(FlatMessagePartsSummary { has_image })
    }
}

#[derive(Default)]
struct FlatMessagePartSummary {
    has_image: bool,
}

impl<'de> Deserialize<'de> for FlatMessagePartSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(FlatMessagePartSummaryVisitor)
    }
}

struct FlatMessagePartSummaryVisitor;

impl<'de> Visitor<'de> for FlatMessagePartSummaryVisitor {
    type Value = FlatMessagePartSummary;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a legacy flat chat message part summary object")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut part = FlatMessagePartSummary::default();
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "type" => {
                    let part_type = map.next_value::<String>()?;
                    part.has_image = matches!(part_type.as_str(), "image" | "image_ref");
                }
                _ => {
                    let _ = map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(part)
    }
}

impl<'de> Deserialize<'de> for FlatMessagesSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(FlatMessagesSummaryVisitor)
    }
}

struct FlatMessagesSummaryVisitor;

impl<'de> Visitor<'de> for FlatMessagesSummaryVisitor {
    type Value = FlatMessagesSummary;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a legacy flat chat session messages array")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut message_count = 0;
        let mut preview = String::new();
        while let Some(message) = seq.next_element::<FlatMessagePreview>()? {
            if message_count == 0 {
                preview = message.preview();
            }
            message_count += 1;
        }
        Ok(FlatMessagesSummary {
            first_message_preview: preview,
            message_count,
        })
    }
}

impl FlatSessionMetaFile {
    fn into_partial_meta(
        self,
        expected_id: &str,
        timestamp_fallback: f64,
    ) -> Result<SessionMeta, String> {
        let id = self.id.unwrap_or_else(|| expected_id.to_string());
        if id != expected_id {
            return Err(invalid_session_error_message_with_id(expected_id));
        }
        let permission_mode = self
            .permission_mode
            .map(|value| validate_permission_mode(&value))
            .transpose()
            .map_err(|_| invalid_session_error_message_with_id(expected_id))?
            .unwrap_or_else(|| crate::permission::PermissionMode::Edit.as_str().to_string());
        validate_meta(
            SessionMeta {
                id,
                worktree_path: self.worktree_path.unwrap_or_default(),
                state: self.state.unwrap_or(SessionState::Active),
                created_at: self.created_at.unwrap_or(timestamp_fallback),
                updated_at: self.updated_at.unwrap_or(timestamp_fallback),
                agent_session_id: self.agent_session_id,
                context_carry: self.context_carry,
                permission_mode,
                plan_mode: self.plan_mode,
                selected_model: self.selected_model,
                permission_profile_id: self.permission_profile_id,
                backend_id: self.backend_id,
                workflow_step_session: self.workflow_step_session,
                first_message_preview: self.first_message_preview,
                message_count: self.message_count,
                body_format_version: SESSION_BODY_FORMAT_VERSION,
            },
            expected_id,
        )
    }
}

impl FileSessionStorage {
    pub(super) fn read_flat_session_file(
        &self,
        path: &Path,
        expected_id: &str,
    ) -> Result<ChatSession, String> {
        let file = std::fs::File::open(path).map_err(|_| invalid_session_error_message())?;
        let mut session: ChatSession = serde_json::from_reader(BufReader::new(file))
            .map_err(|_| invalid_session_error_message_with_id(expected_id))?;
        if session.id != expected_id {
            return Err(invalid_session_error_message_with_id(expected_id));
        }
        session.permission_mode = validate_permission_mode(&session.permission_mode)
            .map_err(|_| invalid_session_error_message_with_id(expected_id))?;
        Ok(session)
    }

    pub(super) fn read_legacy_flat_meta(
        &self,
        app_data_dir: &Path,
        path: &Path,
        expected_id: &str,
    ) -> Result<SessionMeta, String> {
        let sidecar = legacy_meta_file(app_data_dir, expected_id)?;
        if sidecar.exists() {
            match self.read_meta_sidecar(&sidecar, expected_id) {
                Ok(meta) => return Ok(meta),
                Err(err) => {
                    log::warn!(
                        "Ignoring invalid legacy session meta sidecar for {expected_id}: {err}"
                    );
                    let _ = std::fs::remove_file(&sidecar);
                }
            }
        }

        let meta = self.read_flat_session_meta_file(path, expected_id)?;
        if let Err(err) = write_json_pretty_atomic(&sidecar, &meta, "legacy session meta") {
            log::warn!("Failed to write legacy session meta sidecar for {expected_id}: {err}");
        }
        Ok(meta)
    }

    pub(super) fn read_meta_sidecar(
        &self,
        path: &Path,
        expected_id: &str,
    ) -> Result<SessionMeta, String> {
        let file = std::fs::File::open(path).map_err(|_| invalid_session_error_message())?;
        let meta: SessionMeta = serde_json::from_reader(BufReader::new(file))
            .map_err(|_| invalid_session_error_message_with_id(expected_id))?;
        validate_meta(meta, expected_id)
    }

    pub(super) fn read_flat_session_meta_file(
        &self,
        path: &Path,
        expected_id: &str,
    ) -> Result<SessionMeta, String> {
        let file = std::fs::File::open(path).map_err(|_| invalid_session_error_message())?;
        let flat: FlatSessionMetaFile = serde_json::from_reader(BufReader::new(file))
            .map_err(|_| invalid_session_error_message_with_id(expected_id))?;
        flat.into_partial_meta(expected_id, file_timestamp_fallback(path))
    }

    pub(super) fn read_meta_from_dir(
        &self,
        dir: &Path,
        expected_id: &str,
    ) -> Result<SessionMeta, String> {
        let file = std::fs::File::open(meta_file_in_dir(dir))
            .map_err(|_| invalid_session_error_message_with_id(expected_id))?;
        let meta: SessionMeta = serde_json::from_reader(BufReader::new(file))
            .map_err(|_| invalid_session_error_message_with_id(expected_id))?;
        validate_meta(meta, expected_id)
    }
}
