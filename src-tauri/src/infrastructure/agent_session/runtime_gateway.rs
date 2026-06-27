use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::domain::agent_session::PermissionMode;
use crate::infrastructure::agent_session::runtime;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::session::{
    AgentStreamResyncReadModel, SessionStore, StreamResyncSnapshot,
};

pub type AgentImageAttachment = runtime::ImageAttachment;
pub type AgentSendMessageResponse = runtime::SendMessageResponse;

pub struct AgentRuntimeGateway<'a> {
    pub app: &'a tauri::AppHandle,
    pub branch_diff_context: &'a Arc<dyn BranchDiffContextPort>,
    pub session_store: &'a Arc<SessionStore>,
    pub registry: &'a Arc<runtime::AgentBackendRegistry>,
    pub handles: &'a Arc<Mutex<runtime::AgentProcessMap>>,
}

pub struct AgentStreamResyncRuntimeReadModel {
    session_store: Arc<SessionStore>,
    handles: Arc<Mutex<runtime::AgentProcessMap>>,
    data_dir: std::path::PathBuf,
}

impl AgentStreamResyncRuntimeReadModel {
    pub fn new(
        session_store: Arc<SessionStore>,
        handles: Arc<Mutex<runtime::AgentProcessMap>>,
        data_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            session_store,
            handles,
            data_dir,
        }
    }
}

#[async_trait]
impl AgentStreamResyncReadModel for AgentStreamResyncRuntimeReadModel {
    async fn resync_streaming_message(
        &self,
        session_id: &str,
        message_id: &str,
        since_seq: u64,
    ) -> Result<Option<StreamResyncSnapshot>, String> {
        runtime::resync_streaming_message_internal_with_data_dir(
            &self.session_store,
            &self.handles,
            &self.data_dir,
            session_id,
            message_id,
            since_seq,
        )
        .await
    }
}

pub struct AgentRuntimeSendRequest {
    pub chat_session_id: Option<String>,
    pub worktree_path: String,
    pub content: String,
    pub permission_mode: PermissionMode,
    pub plan_mode: bool,
    pub backend_id: Option<String>,
    pub model_id: Option<String>,
    pub images: Option<Vec<AgentImageAttachment>>,
    pub mentions: Option<Vec<crate::domain::code::MentionReference>>,
    pub editor_context: Option<runtime::AgentEditorContext>,
    pub client_sent_at_ms: Option<f64>,
    pub request_received_at_ms: Option<f64>,
}

impl AgentRuntimeGateway<'_> {
    pub async fn send_message(
        self,
        req: AgentRuntimeSendRequest,
    ) -> Result<AgentSendMessageResponse, String> {
        runtime::send_agent_message_internal(
            self.app,
            Some(Arc::clone(self.branch_diff_context)),
            self.session_store,
            self.registry,
            self.handles,
            req.chat_session_id,
            req.worktree_path,
            req.content,
            req.permission_mode,
            req.plan_mode,
            req.backend_id,
            req.model_id,
            req.images,
            req.mentions,
            req.editor_context,
            req.client_sent_at_ms,
            req.request_received_at_ms,
        )
        .await
    }
}
