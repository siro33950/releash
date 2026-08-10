//! Canonical agent-session operation payload conversion.
//!
//! Durable operation drivers live in the usecase layer. This gateway owns the
//! versioned JSON shape and translates it to and from the inner command
//! language used by those drivers.

use serde::{Deserialize, Serialize};

use crate::domain::local_event::{SafeOperationFailure, SessionOperationFailureKind};
use crate::usecase::agent_session::operation::{
    CanonicalSendCommandCodec, DecodedSendCommand, DecodedSendTarget,
};
#[cfg(test)]
use crate::usecase::agent_session::runtime::{
    DurableWorkflowSendError, DurableWorkflowSendPayloadEncoder, DurableWorkflowTurnRequest,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CanonicalSendTargetV1 {
    Direct {
        chat_session_id: Option<String>,
        worktree_path: String,
    },
    WorkflowApproval {
        execution_id: String,
    },
    WorkflowTurn {
        chat_session_id: String,
        base_system_prompt: Option<String>,
        workflow_instructions: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CanonicalSendCommandV1 {
    pub target: CanonicalSendTargetV1,
    pub content: String,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub backend_id: Option<String>,
    pub model_id: Option<String>,
    pub images: Vec<crate::usecase::agent_session::session::ImageAttachment>,
    pub mentions: Vec<crate::adaptor::protocol::mention::MentionReferenceInput>,
    pub editor_context: Option<crate::usecase::agent_session::runtime::usecase::AgentEditorContext>,
}

fn invalid_payload(label: &str) -> SafeOperationFailure {
    SafeOperationFailure::new(
        SessionOperationFailureKind::PersistFailure,
        true,
        label,
        uuid::Uuid::new_v4().to_string(),
    )
}

#[derive(Debug, Default)]
pub(crate) struct CanonicalSendCommandCodecV1;

impl CanonicalSendCommandCodec for CanonicalSendCommandCodecV1 {
    fn decode(&self, canonical_payload: &str) -> Result<DecodedSendCommand, SafeOperationFailure> {
        let command: CanonicalSendCommandV1 = serde_json::from_str(canonical_payload)
            .map_err(|_| invalid_payload("The exact send payload is incompatible."))?;
        let permission_mode =
            crate::domain::agent_session::PermissionMode::parse(&command.permission_mode)
                .map_err(|_| invalid_payload("The permission mode is invalid."))?;
        let target = match command.target {
            CanonicalSendTargetV1::Direct {
                chat_session_id,
                worktree_path,
            } => DecodedSendTarget::Direct {
                chat_session_id,
                worktree_path,
            },
            CanonicalSendTargetV1::WorkflowApproval { execution_id } => {
                DecodedSendTarget::WorkflowApproval { execution_id }
            }
            CanonicalSendTargetV1::WorkflowTurn {
                chat_session_id,
                base_system_prompt,
                workflow_instructions,
            } => DecodedSendTarget::WorkflowTurn {
                chat_session_id,
                base_system_prompt,
                workflow_instructions,
            },
        };
        Ok(DecodedSendCommand {
            target,
            content: command.content,
            permission_mode,
            plan_mode: command.plan_mode,
            backend_id: command.backend_id,
            model_id: command.model_id,
            images: command.images,
            mentions: crate::adaptor::protocol::mention::into_domain_vec(command.mentions),
            editor_context: command.editor_context,
        })
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct CanonicalWorkflowSendPayloadEncoder;

#[cfg(test)]
impl DurableWorkflowSendPayloadEncoder for CanonicalWorkflowSendPayloadEncoder {
    fn encode(
        &self,
        request: &DurableWorkflowTurnRequest,
        plan_mode: bool,
    ) -> Result<String, DurableWorkflowSendError> {
        serde_json::to_string(&CanonicalSendCommandV1 {
            target: CanonicalSendTargetV1::WorkflowTurn {
                chat_session_id: request.session_id.clone(),
                base_system_prompt: request.base_system_prompt.clone(),
                workflow_instructions: request.workflow_instructions.clone(),
            },
            content: request.content.clone(),
            permission_mode: request.permission_mode.as_str().to_string(),
            plan_mode,
            backend_id: None,
            model_id: None,
            images: Vec::new(),
            mentions: Vec::new(),
            editor_context: None,
        })
        .map_err(|_| DurableWorkflowSendError::PayloadEncoding)
    }
}
