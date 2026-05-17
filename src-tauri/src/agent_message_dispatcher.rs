use std::sync::Arc;

use tokio::sync::Mutex;

use crate::backends::{AgentBackendRegistry, ImageAttachment};
use crate::permission::PermissionMode;
use crate::session::errors::session_target_rejected;
use crate::session::{resolve_data_dir, SessionStore};

pub struct AgentMessageDispatchRequest {
    pub chat_session_id: Option<String>,
    pub worktree_path: String,
    pub content: String,
    pub permission_mode: PermissionMode,
    pub backend_id: Option<String>,
    pub images: Option<Vec<ImageAttachment>>,
    pub mentions: Option<Vec<crate::file_mention::MentionReference>>,
}

pub struct AgentMessageDispatchContext<'a> {
    pub app: &'a tauri::AppHandle,
    pub session_store: &'a Arc<SessionStore>,
    pub registry: &'a Arc<AgentBackendRegistry>,
    pub handles: &'a Arc<Mutex<crate::agent_sdk::AgentProcessMap>>,
}

pub async fn dispatch_agent_message(
    context: AgentMessageDispatchContext<'_>,
    req: AgentMessageDispatchRequest,
) -> Result<crate::agent_sdk::SendMessageResponse, String> {
    let data_dir = resolve_data_dir(context.app)?;
    let workflow_step_target = match req.chat_session_id.as_deref() {
        Some(session_id) => context
            .session_store
            .get_session(&data_dir, session_id)?
            .map(|session| session.workflow_step_session)
            .unwrap_or(false),
        None => false,
    };
    let response = crate::agent_sdk::send_agent_message_internal(
        context.app,
        context.session_store,
        context.registry,
        context.handles,
        req.chat_session_id,
        req.worktree_path.clone(),
        req.content,
        req.permission_mode,
        req.backend_id,
        req.images,
        req.mentions,
    )
    .await
    .map_err(|err| redact_dispatch_error(workflow_step_target, err))?;
    Ok(response)
}

fn redact_dispatch_error(workflow_step_target: bool, err: String) -> String {
    if workflow_step_target {
        session_target_rejected()
    } else {
        err
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_step_dispatch_spawn_error_is_redacted() {
        let err = redact_dispatch_error(
            true,
            "Failed to spawn runtime for /repo with agent_session_id=sdk-secret".to_string(),
        );

        assert_eq!(err, crate::session::errors::SESSION_TARGET_REJECTED);
        assert!(!err.contains("/repo"));
        assert!(!err.contains("sdk-secret"));
    }

    #[test]
    fn non_workflow_dispatch_error_preserves_existing_message() {
        let err = redact_dispatch_error(false, "regular failure".to_string());

        assert_eq!(err, "regular failure");
    }
}
