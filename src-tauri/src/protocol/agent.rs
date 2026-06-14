use serde::{Deserialize, Serialize};

use crate::adaptor::protocol::mention::MentionReferenceInput;
use crate::infrastructure::agent_session::runtime::{AgentEditorContext, ImageAttachment};
use crate::permission::{InvalidPermissionMode, PermissionMode};
use crate::usecase::agent_session::session::{GetSessionResponse, QueuedAgentTurn, SessionSummary};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Running,
    Done,
    Error,
    Waiting,
}

impl From<crate::usecase::agent_session::status::AgentState> for AgentState {
    fn from(state: crate::usecase::agent_session::status::AgentState) -> Self {
        match state {
            crate::usecase::agent_session::status::AgentState::Running => Self::Running,
            crate::usecase::agent_session::status::AgentState::Done => Self::Done,
            crate::usecase::agent_session::status::AgentState::Error => Self::Error,
            crate::usecase::agent_session::status::AgentState::Waiting => Self::Waiting,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStateSync {
    pub worktree_path: String,
    pub state: AgentState,
    pub exit_code: Option<i32>,
    pub timestamp: f64,
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pty_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendListRequest {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendListResponse {
    pub backends: Vec<BackendInfoMsg>,
    pub default_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendInfoMsg {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub available_models: Vec<ModelInfoMsg>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfoMsg {
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSupportedCommandMsg {
    pub name: String,
    pub description: String,
    #[serde(
        rename = "argumentHint",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSupportedCommandsUpdated {
    pub chat_session_id: String,
    pub commands: Vec<AgentSupportedCommandMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionStartRequest {
    pub worktree_path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    /// 抽象パーミッションモード（ask / edit / full）。
    /// リモート UI で選択された permission_mode をセッション開始時にセッション保存層へ反映する。
    /// 欠落・対象外値は WebSocket ハンドラで InvalidPermissionMode として拒否する。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub permission_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionStartResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionsRequest {
    pub worktree_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionsResponse {
    pub success: bool,
    pub worktree_path: String,
    pub sessions: Vec<SessionSummary>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_session: Option<GetSessionResponse>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionGetRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionGetResponse {
    pub success: bool,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session: Option<GetSessionResponse>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessageRequest {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    pub worktree_path: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(default)]
    pub images: Vec<ImageAttachment>,
    #[serde(default)]
    pub mentions: Vec<MentionReferenceInput>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub editor_context: Option<AgentEditorContext>,
}

/// `AgentSessionStartRequest` の typed 境界変換結果。
/// WS 受信直後に [`TryFrom`] 経由で生成され、handler 経路の入口で
/// `permission_mode` が抽象 [`PermissionMode`] に確定済みであることを型で保証する。
/// wire 型は欠落・対象外値を許す `Option<String>` のまま保つ（serde 失敗で
/// WS 全体デコードが落ちる事態を避ける）が、境界の検証はこの型へ
/// 変換できるかどうかで一段化する（Spec issues-947）。
#[derive(Debug, Clone)]
pub struct AgentSessionStartHandlerRequest {
    pub worktree_path: String,
    pub backend_id: Option<String>,
    pub permission_mode: PermissionMode,
}

impl TryFrom<&AgentSessionStartRequest> for AgentSessionStartHandlerRequest {
    type Error = InvalidPermissionMode;

    fn try_from(req: &AgentSessionStartRequest) -> Result<Self, Self::Error> {
        let value = req.permission_mode.as_deref().unwrap_or("");
        let permission_mode = PermissionMode::parse(value)?;
        Ok(Self {
            worktree_path: req.worktree_path.clone(),
            backend_id: req.backend_id.clone(),
            permission_mode,
        })
    }
}

/// `AgentMessageRequest` の typed 境界変換結果。役割は
/// [`AgentSessionStartHandlerRequest`] と同じ。
#[derive(Debug, Clone)]
pub struct AgentMessageHandlerRequest {
    pub session_id: Option<String>,
    pub worktree_path: String,
    pub content: String,
    pub permission_mode: PermissionMode,
    pub backend_id: Option<String>,
    pub images: Vec<ImageAttachment>,
    pub mentions: Vec<MentionReferenceInput>,
    pub editor_context: Option<AgentEditorContext>,
}

impl TryFrom<&AgentMessageRequest> for AgentMessageHandlerRequest {
    type Error = InvalidPermissionMode;

    fn try_from(req: &AgentMessageRequest) -> Result<Self, Self::Error> {
        let value = req.permission_mode.as_deref().unwrap_or("");
        let permission_mode = PermissionMode::parse(value)?;
        Ok(Self {
            session_id: req.session_id.clone(),
            worktree_path: req.worktree_path.clone(),
            content: req.content.clone(),
            permission_mode,
            backend_id: req.backend_id.clone(),
            images: req.images.clone(),
            mentions: req.mentions.clone(),
            editor_context: req.editor_context.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessageResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub human_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub queued_turn_id: Option<String>,
    #[serde(default)]
    pub pending_queue: Vec<QueuedAgentTurn>,
    #[serde(default)]
    pub pending_queue_count: usize,
    #[serde(default)]
    pub sessions: Vec<SessionSummary>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInterruptRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInterruptResponse {
    pub success: bool,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentQueueCancelRequest {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub queued_turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentQueueCancelResponse {
    pub success: bool,
    pub session_id: String,
    pub canceled_count: usize,
    #[serde(default)]
    pub pending_queue: Vec<QueuedAgentTurn>,
    #[serde(default)]
    pub pending_queue_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSlashCommandsRequest {
    pub worktree_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSlashCommandEntry {
    pub name: String,
    pub description: String,
    #[serde(
        rename = "argumentHint",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub argument_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSlashCommandsResponse {
    pub success: bool,
    pub worktree_path: String,
    #[serde(default)]
    pub commands: Vec<AgentSlashCommandEntry>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMentionFilesRequest {
    pub request_id: String,
    pub worktree_path: String,
    pub query: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMentionFilesResponse {
    pub success: bool,
    pub request_id: String,
    pub worktree_path: String,
    pub query: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentImagePrepareRequest {
    pub request_id: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentImagePrepareResponse {
    pub success: bool,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub attachment: Option<ImageAttachment>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPermissionResponseRequest {
    pub session_id: String,
    pub request_id: String,
    pub behavior: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub updated_input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPermissionResponseResponse {
    pub success: bool,
    pub session_id: String,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentModelSetRequest {
    pub session_id: String,
    /// モデルは必須（モデル未選択状態は廃止）。リモートクライアントはデスクトップ同梱物のため
    /// 互換問題はない。
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentModelSetResponse {
    pub success: bool,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPermissionModeSetRequest {
    pub session_id: String,
    pub permission_mode: String,
}

#[derive(Debug, Clone)]
pub struct AgentPermissionModeSetHandlerRequest {
    pub session_id: String,
    pub permission_mode: PermissionMode,
}

impl TryFrom<&AgentPermissionModeSetRequest> for AgentPermissionModeSetHandlerRequest {
    type Error = InvalidPermissionMode;

    fn try_from(req: &AgentPermissionModeSetRequest) -> Result<Self, Self::Error> {
        let permission_mode = PermissionMode::parse(&req.permission_mode)?;
        Ok(Self {
            session_id: req.session_id.clone(),
            permission_mode,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPermissionModeSetResponse {
    pub success: bool,
    pub session_id: String,
    pub permission_mode: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentStreamPartMsg {
    Thinking {
        content: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    Text {
        content: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    ToolUse {
        tool: String,
        input: serde_json::Value,
        id: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    ToolResult {
        content: String,
        #[serde(rename = "isError")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "toolUseId")]
        tool_use_id: Option<String>,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    Error {
        content: String,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    Permission {
        request: serde_json::Value,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        answers: Option<serde_json::Value>,
        #[serde(
            skip_serializing_if = "Option::is_none",
            default,
            rename = "parentToolUseId"
        )]
        parent_tool_use_id: Option<String>,
    },
    TaskStatus {
        #[serde(rename = "taskToolUseId")]
        task_tool_use_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        summary: Option<String>,
    },
    SystemNotification {
        #[serde(rename = "notificationType")]
        notification_type: String,
        status: String,
        label: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "hookId")]
        hook_id: Option<String>,
    },
    Image {
        data: String,
        #[serde(rename = "mediaType")]
        media_type: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStreamSync {
    pub session_id: String,
    pub message_id: String,
    pub parts: Vec<AgentStreamPartMsg>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_agent_state_sync() {
        let sync = AgentStateSync {
            worktree_path: "/repo".to_string(),
            state: AgentState::Running,
            exit_code: None,
            timestamp: 1234567890.0,
            session_id: None,
            pty_id: None,
        };
        let json = serde_json::to_string(&sync).unwrap();
        let back: AgentStateSync = serde_json::from_str(&json).unwrap();
        assert_eq!(back.state, AgentState::Running);
        assert_eq!(back.worktree_path, "/repo");
    }

    #[test]
    fn agent_state_serializes_snake_case() {
        let json = serde_json::to_string(&AgentState::Running).unwrap();
        assert_eq!(json, "\"running\"");
        let json = serde_json::to_string(&AgentState::Done).unwrap();
        assert_eq!(json, "\"done\"");
        let json = serde_json::to_string(&AgentState::Error).unwrap();
        assert_eq!(json, "\"error\"");
        let json = serde_json::to_string(&AgentState::Waiting).unwrap();
        assert_eq!(json, "\"waiting\"");
    }

    #[test]
    fn pty_id_none_is_skipped_in_serialization() {
        let sync = AgentStateSync {
            worktree_path: "/repo".to_string(),
            state: AgentState::Running,
            exit_code: None,
            timestamp: 1000.0,
            session_id: None,
            pty_id: None,
        };
        let json = serde_json::to_string(&sync).unwrap();
        assert!(!json.contains("pty_id"));
    }

    #[test]
    fn agent_session_start_handler_request_accepts_abstract_modes() {
        for value in ["ask", "edit", "full"] {
            let req = AgentSessionStartRequest {
                worktree_path: "/repo".to_string(),
                backend_id: Some("claude".to_string()),
                permission_mode: Some(value.to_string()),
            };
            let typed: AgentSessionStartHandlerRequest = (&req).try_into().unwrap();
            assert_eq!(typed.permission_mode, PermissionMode::parse(value).unwrap());
            assert_eq!(typed.worktree_path, "/repo");
            assert_eq!(typed.backend_id.as_deref(), Some("claude"));
        }
    }

    #[test]
    fn agent_session_start_handler_request_rejects_invalid_permission() {
        for value in [
            None,
            Some(""),
            Some("acceptEdits"),
            Some("bypassPermissions"),
            Some("plan"),
            Some("default"),
            Some("unknown"),
        ] {
            let req = AgentSessionStartRequest {
                worktree_path: "/repo".to_string(),
                backend_id: None,
                permission_mode: value.map(str::to_string),
            };
            let err = AgentSessionStartHandlerRequest::try_from(&req).unwrap_err();
            assert!(
                err.to_string().contains("ask, edit, full"),
                "{value:?} must be rejected with allowed list, got {err}"
            );
        }
    }

    #[test]
    fn agent_message_handler_request_accepts_abstract_modes() {
        for value in ["ask", "edit", "full"] {
            let req = AgentMessageRequest {
                session_id: Some("s1".to_string()),
                worktree_path: "/repo".to_string(),
                content: "hi".to_string(),
                permission_mode: Some(value.to_string()),
                backend_id: None,
                images: Vec::new(),
                mentions: Vec::new(),
                editor_context: None,
            };
            let typed: AgentMessageHandlerRequest = (&req).try_into().unwrap();
            assert_eq!(typed.permission_mode, PermissionMode::parse(value).unwrap());
            assert_eq!(typed.content, "hi");
        }
    }

    #[test]
    fn agent_message_handler_request_rejects_invalid_permission() {
        for value in [
            None,
            Some(""),
            Some("acceptEdits"),
            Some("bypassPermissions"),
            Some("plan"),
            Some("default"),
            Some("unknown"),
        ] {
            let req = AgentMessageRequest {
                session_id: None,
                worktree_path: "/repo".to_string(),
                content: "hi".to_string(),
                permission_mode: value.map(str::to_string),
                backend_id: None,
                images: Vec::new(),
                mentions: Vec::new(),
                editor_context: None,
            };
            let err = AgentMessageHandlerRequest::try_from(&req).unwrap_err();
            assert!(
                err.to_string().contains("ask, edit, full"),
                "{value:?} must be rejected with allowed list, got {err}"
            );
        }
    }

    #[test]
    fn pty_id_some_is_serialized() {
        let sync = AgentStateSync {
            worktree_path: "/repo".to_string(),
            state: AgentState::Running,
            exit_code: None,
            timestamp: 1000.0,
            session_id: None,
            pty_id: Some("7".to_string()),
        };
        let json = serde_json::to_string(&sync).unwrap();
        assert!(json.contains("\"pty_id\":\"7\""));
    }
}
