use crate::app_data_dir::resolve_data_dir;
use crate::infrastructure::agent_session::runtime_gateway::{
    AgentImageAttachment, AgentRuntimeGateway, AgentRuntimeSendRequest, AgentSendMessageResponse,
};
use crate::permission::PermissionMode;
use crate::usecase::agent_session::session::errors::session_target_rejected;

pub struct AgentMessageDispatchRequest {
    pub chat_session_id: Option<String>,
    pub worktree_path: String,
    pub content: String,
    pub permission_mode: PermissionMode,
    pub backend_id: Option<String>,
    pub images: Option<Vec<AgentImageAttachment>>,
    pub mentions: Option<Vec<crate::domain::code::MentionReference>>,
    pub editor_context: Option<crate::infrastructure::agent_session::runtime::AgentEditorContext>,
}

pub struct AgentMessageDispatchContext<'a> {
    pub gateway: AgentRuntimeGateway<'a>,
}

pub async fn dispatch_agent_message(
    context: AgentMessageDispatchContext<'_>,
    req: AgentMessageDispatchRequest,
) -> Result<AgentSendMessageResponse, String> {
    let data_dir = resolve_data_dir(context.gateway.app)?;
    let workflow_step_target = match req.chat_session_id.as_deref() {
        Some(session_id) => context
            .gateway
            .session_store
            .get_session(&data_dir, session_id)?
            .map(|session| session.workflow_step_session)
            .unwrap_or(false),
        None => false,
    };
    let response = context
        .gateway
        .send_message(AgentRuntimeSendRequest {
            chat_session_id: req.chat_session_id,
            worktree_path: req.worktree_path.clone(),
            content: req.content,
            permission_mode: req.permission_mode,
            backend_id: req.backend_id,
            images: req.images,
            mentions: req.mentions,
            editor_context: req.editor_context,
        })
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

        assert_eq!(
            err,
            crate::usecase::agent_session::session::errors::SESSION_TARGET_REJECTED
        );
        assert!(!err.contains("/repo"));
        assert!(!err.contains("sdk-secret"));
    }

    #[test]
    fn non_workflow_dispatch_error_preserves_existing_message() {
        let err = redact_dispatch_error(false, "regular failure".to_string());

        assert_eq!(err, "regular failure");
    }
}
