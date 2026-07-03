use crate::usecase::agent_session::session::{now_timestamp, ImageAttachment, QueuedAgentTurn};

use super::usecase::AgentEditorContext;

#[derive(Debug, Clone)]
pub(crate) struct QueuedTurnInput {
    pub id: String,
    pub content: String,
    pub created_at: f64,
    pub permission_mode: crate::domain::agent_session::PermissionMode,
    pub plan_mode: bool,
    pub permission_profile_id: Option<String>,
    pub images: Vec<ImageAttachment>,
    pub worktree_path: String,
    pub mentions: Vec<crate::domain::code::MentionReference>,
    pub editor_context: Option<AgentEditorContext>,
    pub existing_human_message_id: Option<String>,
    pub existing_agent_message_id: Option<String>,
}

impl QueuedTurnInput {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        content: String,
        permission_mode: crate::domain::agent_session::PermissionMode,
        plan_mode: bool,
        permission_profile_id: Option<String>,
        images: Vec<ImageAttachment>,
        worktree_path: String,
        mentions: Vec<crate::domain::code::MentionReference>,
        editor_context: Option<AgentEditorContext>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            content,
            created_at: now_timestamp(),
            permission_mode,
            plan_mode,
            permission_profile_id,
            images,
            worktree_path,
            mentions,
            editor_context,
            existing_human_message_id: None,
            existing_agent_message_id: None,
        }
    }
}

impl From<&QueuedTurnInput> for QueuedAgentTurn {
    fn from(input: &QueuedTurnInput) -> Self {
        Self {
            id: input.id.clone(),
            content_preview: input.content.chars().take(120).collect(),
            created_at: input.created_at,
            permission_mode: input.permission_mode.as_str().to_string(),
            image_count: input.images.len(),
        }
    }
}
