use std::sync::Arc;

use tokio::sync::Mutex;

use crate::infrastructure::agent_session::runtime;
use crate::permission::PermissionMode;
use crate::usecase::agent_session::session::SessionStore;

pub type AgentImageAttachment = runtime::ImageAttachment;
pub type AgentSendMessageResponse = runtime::SendMessageResponse;

pub struct AgentRuntimeGateway<'a> {
    pub app: &'a tauri::AppHandle,
    pub session_store: &'a Arc<SessionStore>,
    pub registry: &'a Arc<runtime::AgentBackendRegistry>,
    pub handles: &'a Arc<Mutex<runtime::AgentProcessMap>>,
}

pub struct AgentRuntimeSendRequest {
    pub chat_session_id: Option<String>,
    pub worktree_path: String,
    pub content: String,
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub backend_id: Option<String>,
    pub images: Option<Vec<AgentImageAttachment>>,
    pub mentions: Option<Vec<crate::domain::code::MentionReference>>,
    pub editor_context: Option<runtime::AgentEditorContext>,
}

impl AgentRuntimeGateway<'_> {
    pub async fn send_message(
        self,
        req: AgentRuntimeSendRequest,
    ) -> Result<AgentSendMessageResponse, String> {
        runtime::send_agent_message_internal(
            self.app,
            self.session_store,
            self.registry,
            self.handles,
            req.chat_session_id,
            req.worktree_path,
            req.content,
            req.permission_mode,
            req.plan_mode,
            req.backend_id,
            req.images,
            req.mentions,
            req.editor_context,
        )
        .await
    }
}
