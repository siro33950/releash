mod store;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Manager, State};

pub use crate::workflow::state::WorkflowState;
pub use store::SessionStore;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
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
        /// Base64-encoded image data
        data: String,
        /// MIME type (e.g. "image/png", "image/jpeg")
        #[serde(rename = "mediaType")]
        media_type: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    Human,
    Agent,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Idle,
    Done,
    Error,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActivityEntry {
    ToolUse {
        tool: String,
        input: serde_json::Value,
        id: String,
    },
    ToolResult {
        content: String,
        #[serde(rename = "isError")]
        is_error: bool,
        #[serde(skip_serializing_if = "Option::is_none", default, rename = "toolUseId")]
        tool_use_id: Option<String>,
    },
    PermissionResult {
        #[serde(rename = "toolName")]
        tool_name: String,
        status: String,
        summary: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub activities: Option<Vec<ActivityEntry>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parts: Option<Vec<MessagePart>>,
    pub timestamp: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mentions: Option<Vec<crate::file_mention::MentionReference>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    pub id: String,
    pub worktree_path: String,
    pub messages: Vec<ChatMessage>,
    pub state: SessionState,
    pub created_at: f64,
    pub updated_at: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_session_id: Option<String>,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub selected_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub workflow_state: Option<WorkflowState>,
}

fn default_permission_mode() -> String {
    "acceptEdits".to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetSessionResponse {
    #[serde(flatten)]
    pub session: ChatSession,
    pub turn_phase: crate::agent_sdk::TurnPhase,
    pub available_models: Vec<crate::agent_sdk::ModelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub worktree_path: String,
    pub state: SessionState,
    pub created_at: f64,
    pub updated_at: f64,
    pub first_message: String,
    pub message_count: usize,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub agent_session_id: Option<String>,
    pub permission_mode: String,
}

impl ChatSession {
    pub fn to_summary(&self) -> SessionSummary {
        let first_message = self
            .messages
            .first()
            .map(|m| {
                let content = if m.content.is_empty() {
                    if m.parts.as_ref().is_some_and(|parts| {
                        parts.iter().any(|p| matches!(p, MessagePart::Image { .. }))
                    }) {
                        "[Image]".to_string()
                    } else {
                        m.content.clone()
                    }
                } else {
                    m.content.clone()
                };
                match content.char_indices().nth(100) {
                    Some((byte_pos, _)) => format!("{}…", &content[..byte_pos]),
                    None => content,
                }
            })
            .unwrap_or_default();

        SessionSummary {
            id: self.id.clone(),
            worktree_path: self.worktree_path.clone(),
            state: self.state.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            first_message,
            message_count: self.messages.len(),
            agent_session_id: self.agent_session_id.clone(),
            permission_mode: self.permission_mode.clone(),
        }
    }
}

pub(crate) fn resolve_data_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))
}

pub(crate) fn now_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn parts_to_legacy(
    parts: &[MessagePart],
) -> (String, Option<String>, Option<Vec<ActivityEntry>>) {
    let mut content = String::new();
    let mut thinking = String::new();
    let mut activities: Vec<ActivityEntry> = Vec::new();
    for part in parts {
        match part {
            MessagePart::Text { content: c, .. } => content.push_str(c),
            MessagePart::Error { content: c, .. } => content.push_str(c),
            MessagePart::Thinking { content: c, .. } => thinking.push_str(c),
            MessagePart::ToolUse {
                tool, input, id, ..
            } => {
                activities.push(ActivityEntry::ToolUse {
                    tool: tool.clone(),
                    input: input.clone(),
                    id: id.clone(),
                });
            }
            MessagePart::ToolResult {
                content: c,
                is_error,
                tool_use_id,
                ..
            } => {
                activities.push(ActivityEntry::ToolResult {
                    content: c.clone(),
                    is_error: *is_error,
                    tool_use_id: tool_use_id.clone(),
                });
            }
            MessagePart::Permission {
                request,
                status,
                answers,
                ..
            } => {
                if status != "pending" {
                    let tool_name = request
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let summary = answers
                        .as_ref()
                        .and_then(|a| a.as_object())
                        .map(|obj| {
                            obj.values()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_else(|| status.clone());
                    activities.push(ActivityEntry::PermissionResult {
                        tool_name,
                        status: status.clone(),
                        summary,
                    });
                }
            }
            MessagePart::TaskStatus { .. } => {}
            MessagePart::SystemNotification { .. } => {}
            MessagePart::Image { .. } => {}
        }
    }
    let thinking = if thinking.is_empty() {
        None
    } else {
        Some(thinking)
    };
    let activities = if activities.is_empty() {
        None
    } else {
        Some(activities)
    };
    (content, thinking, activities)
}

#[tauri::command]
pub fn list_sessions(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<SessionSummary>, String> {
    let data_dir = resolve_data_dir(&app)?;
    state.list_sessions(&data_dir, &worktree_path)
}

/// Internal (non-command) version of create_session, callable from agent_sdk.
pub fn create_session_internal(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    worktree_path: &str,
) -> Result<ChatSession, String> {
    let now = now_timestamp();
    let session = ChatSession {
        id: uuid::Uuid::new_v4().to_string(),
        worktree_path: worktree_path.to_string(),
        messages: Vec::new(),
        state: SessionState::Active,
        created_at: now,
        updated_at: now,
        agent_session_id: None,
        permission_mode: default_permission_mode(),
        selected_model: None,
        workflow_state: None,
    };
    session_store.save_session(data_dir, &session)?;
    Ok(session)
}

/// Internal (non-command) version of add_message, callable from agent_sdk.
pub fn add_message_internal(
    session_store: &SessionStore,
    data_dir: &std::path::Path,
    session_id: &str,
    role: MessageRole,
    content: &str,
    parts: Option<Vec<MessagePart>>,
    mentions: Option<Vec<crate::file_mention::MentionReference>>,
) -> Result<ChatMessage, String> {
    let mut session = session_store
        .get_session(data_dir, session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    let now = now_timestamp();
    let message = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content: content.to_string(),
        thinking: None,
        activities: None,
        parts,
        timestamp: now,
        mentions,
    };
    session.messages.push(message.clone());
    session.updated_at = now;
    session_store.save_session(data_dir, &session)?;
    Ok(message)
}

#[tauri::command]
pub fn create_session(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<ChatSession, String> {
    let data_dir = resolve_data_dir(&app)?;
    create_session_internal(&state, &data_dir, &worktree_path)
}

#[tauri::command]
pub fn add_message(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    role: MessageRole,
    content: String,
) -> Result<ChatMessage, String> {
    let data_dir = resolve_data_dir(&app)?;
    add_message_internal(&state, &data_dir, &session_id, role, &content, None, None)
}

#[tauri::command]
pub fn update_session_state(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    new_state: SessionState,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let mut session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    session.state = new_state;
    session.updated_at = now_timestamp();
    state.save_session(&data_dir, &session)?;
    Ok(())
}

#[tauri::command]
pub fn close_session(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let mut session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    session.state = SessionState::Closed;
    session.updated_at = now_timestamp();
    state.save_session(&data_dir, &session)?;
    Ok(())
}

#[tauri::command]
pub fn restore_session(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let mut session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    session.state = SessionState::Idle;
    session.updated_at = now_timestamp();
    state.save_session(&data_dir, &session)?;
    Ok(())
}

#[tauri::command]
pub fn list_closed_sessions(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<Vec<SessionSummary>, String> {
    let data_dir = resolve_data_dir(&app)?;
    state.list_closed_sessions(&data_dir, &worktree_path)
}

#[tauri::command]
pub fn update_session_agent_info(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    agent_session_id: Option<String>,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let mut session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    session.agent_session_id = agent_session_id;
    session.updated_at = now_timestamp();
    state.save_session(&data_dir, &session)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::schema::{Step, StepMode, Workflow};
    use crate::workflow::state::{StepHistoryEntry, TokenUsage, WorkflowExecutionState};

    #[test]
    fn chat_session_to_summary_basic() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: "Hello agent".to_string(),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
        };
        let summary = session.to_summary();
        assert_eq!(summary.id, "s1");
        assert_eq!(summary.first_message, "Hello agent");
        // Verify selected_model not in summary (summary doesn't expose model)
        assert_eq!(summary.message_count, 1);
    }

    #[test]
    fn chat_session_to_summary_truncates_long_message() {
        let long_content = "a".repeat(200);
        let session = ChatSession {
            id: "s2".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: long_content,
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Idle,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
        };
        let summary = session.to_summary();
        assert_eq!(summary.first_message.len(), 100 + "…".len());
        assert!(summary.first_message.ends_with('…'));
    }

    #[test]
    fn chat_session_to_summary_truncates_multibyte_message() {
        // 200 Japanese characters (3 bytes each in UTF-8)
        let long_content = "あ".repeat(200);
        let session = ChatSession {
            id: "s2mb".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![ChatMessage {
                id: "m1".to_string(),
                role: MessageRole::Human,
                content: long_content,
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1000.0,
                mentions: None,
            }],
            state: SessionState::Idle,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
        };
        let summary = session.to_summary();
        // 100 chars of "あ" (300 bytes) + "…" (3 bytes)
        assert_eq!(summary.first_message.chars().count(), 101); // 100 + 1 for "…"
        assert!(summary.first_message.ends_with('…'));
        assert!(summary.first_message.starts_with("あ"));
    }

    #[test]
    fn chat_session_to_summary_empty_messages() {
        let session = ChatSession {
            id: "s3".to_string(),
            worktree_path: "/repo".to_string(),
            messages: Vec::new(),
            state: SessionState::Done,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
        };
        let summary = session.to_summary();
        assert_eq!(summary.first_message, "");
        assert_eq!(summary.message_count, 0);
    }

    #[test]
    fn message_role_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&MessageRole::Human).unwrap(),
            "\"human\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Agent).unwrap(),
            "\"agent\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::System).unwrap(),
            "\"system\""
        );
    }

    #[test]
    fn session_state_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&SessionState::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Idle).unwrap(),
            "\"idle\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Done).unwrap(),
            "\"done\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Error).unwrap(),
            "\"error\""
        );
        assert_eq!(
            serde_json::to_string(&SessionState::Closed).unwrap(),
            "\"closed\""
        );
    }

    #[test]
    fn chat_message_thinking_field_serialization() {
        let msg_with = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "response".to_string(),
            thinking: Some("deep thought".to_string()),
            activities: None,
            parts: None,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg_with).unwrap();
        assert!(json.contains("\"thinking\":\"deep thought\""));
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.thinking, Some("deep thought".to_string()));

        let msg_without = ChatMessage {
            id: "m2".to_string(),
            role: MessageRole::Agent,
            content: "response".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg_without).unwrap();
        assert!(!json.contains("thinking"));
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.thinking, None);
    }

    #[test]
    fn chat_message_without_thinking_field_deserializes() {
        let json = r#"{"id":"m1","role":"agent","content":"hello","timestamp":1000.0}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.thinking, None);
    }

    #[test]
    fn chat_session_roundtrip() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![
                ChatMessage {
                    id: "m1".to_string(),
                    role: MessageRole::Human,
                    content: "Hello".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    timestamp: 1000.0,
                    mentions: None,
                },
                ChatMessage {
                    id: "m2".to_string(),
                    role: MessageRole::Agent,
                    content: "Hi there!".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    timestamp: 1001.0,
                    mentions: None,
                },
            ],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
        };
        let json = serde_json::to_string(&session).unwrap();
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "s1");
        assert_eq!(back.messages.len(), 2);
        assert_eq!(back.messages[0].role, MessageRole::Human);
        assert_eq!(back.messages[1].role, MessageRole::Agent);
    }

    #[test]
    fn chat_session_without_selected_model_deserializes() {
        let json = r#"{"id":"s1","worktreePath":"/repo","messages":[],"state":"active","createdAt":1000.0,"updatedAt":1000.0}"#;
        let session: ChatSession = serde_json::from_str(json).unwrap();
        assert_eq!(session.selected_model, None);
    }

    #[test]
    fn chat_session_roundtrip_with_selected_model() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: Some("claude-opus-4-6".to_string()),
            workflow_state: None,
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("selectedModel"));
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.selected_model, Some("claude-opus-4-6".to_string()));
    }

    #[test]
    fn activity_entry_tool_use_serialization() {
        let entry = ActivityEntry::ToolUse {
            tool: "Read".to_string(),
            input: serde_json::json!({"file_path": "/src/main.ts"}),
            id: "toolu_001".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "tool_use");
        assert_eq!(v["tool"], "Read");
        assert_eq!(v["id"], "toolu_001");
        assert_eq!(v["input"]["file_path"], "/src/main.ts");
    }

    #[test]
    fn activity_entry_tool_result_serialization() {
        let entry = ActivityEntry::ToolResult {
            content: "file contents".to_string(),
            is_error: false,
            tool_use_id: Some("toolu_001".into()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "tool_result");
        assert_eq!(v["content"], "file contents");
        assert_eq!(v["isError"], false);
        assert_eq!(v["toolUseId"], "toolu_001");
    }

    #[test]
    fn activity_entry_permission_result_serialization() {
        let entry = ActivityEntry::PermissionResult {
            tool_name: "Bash".to_string(),
            status: "allowed".to_string(),
            summary: "Bash: allowed".to_string(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "permission_result");
        assert_eq!(v["toolName"], "Bash");
        assert_eq!(v["status"], "allowed");
        assert_eq!(v["summary"], "Bash: allowed");

        let back: ActivityEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn activity_entry_permission_result_backward_compat() {
        // Existing session files without permission_result should still deserialize
        let json = r#"{"type":"tool_use","tool":"Read","input":{},"id":"t1"}"#;
        let entry: ActivityEntry = serde_json::from_str(json).unwrap();
        assert!(matches!(entry, ActivityEntry::ToolUse { .. }));
    }

    #[test]
    fn chat_message_without_activities_field_deserializes() {
        let json = r#"{"id":"m1","role":"agent","content":"hello","timestamp":1000.0}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.activities, None);
    }

    #[test]
    fn chat_message_with_activities_roundtrip() {
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "done".to_string(),
            thinking: None,
            activities: Some(vec![
                ActivityEntry::ToolUse {
                    tool: "Read".to_string(),
                    input: serde_json::json!({}),
                    id: "t1".to_string(),
                },
                ActivityEntry::ToolResult {
                    content: "ok".to_string(),
                    is_error: false,
                    tool_use_id: None,
                },
            ]),
            parts: None,
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.activities.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn message_part_serde_roundtrip() {
        let parts = vec![
            MessagePart::Thinking {
                content: "hmm".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Error {
                content: "something went wrong".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolUse {
                tool: "Read".to_string(),
                input: serde_json::json!({"file_path": "/a.ts"}),
                id: "t1".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::ToolResult {
                content: "ok".to_string(),
                is_error: false,
                tool_use_id: None,
                parent_tool_use_id: None,
            },
            MessagePart::Permission {
                request: serde_json::json!({"request_id": "r1", "tool_name": "Bash"}),
                status: "allowed".to_string(),
                answers: Some(serde_json::json!({"q1": "yes"})),
                parent_tool_use_id: None,
            },
        ];
        let json = serde_json::to_string(&parts).unwrap();
        let back: Vec<MessagePart> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 6);
        assert_eq!(back, parts);
    }

    #[test]
    fn message_part_error_serialization() {
        let part = MessagePart::Error {
            content: "fail".to_string(),
            parent_tool_use_id: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "error");
        assert_eq!(v["content"], "fail");
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn chat_message_with_parts_roundtrip() {
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "hi".to_string(),
            thinking: Some("think".to_string()),
            activities: None,
            parts: Some(vec![
                MessagePart::Thinking {
                    content: "think".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Text {
                    content: "hi".to_string(),
                    parent_tool_use_id: None,
                },
            ]),
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parts.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn old_json_without_parts_deserializes_to_none() {
        let json = r#"{"id":"m1","role":"agent","content":"hello","timestamp":1000.0}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.parts, None);
    }

    #[test]
    fn message_part_permission_without_answers_serializes() {
        let part = MessagePart::Permission {
            request: serde_json::json!({"request_id": "r1"}),
            status: "pending".to_string(),
            answers: None,
            parent_tool_use_id: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(!json.contains("answers"));
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        if let MessagePart::Permission { answers, .. } = back {
            assert_eq!(answers, None);
        } else {
            panic!("Expected Permission variant");
        }
    }

    #[test]
    fn tool_result_without_tool_use_id_deserializes() {
        let json = r#"{"type":"tool_result","content":"ok","isError":false}"#;
        let part: MessagePart = serde_json::from_str(json).unwrap();
        if let MessagePart::ToolResult { tool_use_id, .. } = part {
            assert_eq!(tool_use_id, None);
        } else {
            panic!("Expected ToolResult variant");
        }

        let entry: ActivityEntry = serde_json::from_str(json).unwrap();
        if let ActivityEntry::ToolResult { tool_use_id, .. } = entry {
            assert_eq!(tool_use_id, None);
        } else {
            panic!("Expected ToolResult variant");
        }
    }

    #[test]
    fn task_status_serde_roundtrip() {
        let part = MessagePart::TaskStatus {
            task_tool_use_id: "toolu_task_001".to_string(),
            status: "completed".to_string(),
            description: Some("Search codebase".to_string()),
            summary: Some("Found 3 files".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "task_status");
        assert_eq!(v["taskToolUseId"], "toolu_task_001");
        assert_eq!(v["status"], "completed");
        assert_eq!(v["description"], "Search codebase");
        assert_eq!(v["summary"], "Found 3 files");
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn task_status_without_optional_fields_deserializes() {
        let json = r#"{"type":"task_status","taskToolUseId":"t1","status":"started"}"#;
        let part: MessagePart = serde_json::from_str(json).unwrap();
        if let MessagePart::TaskStatus {
            task_tool_use_id,
            status,
            description,
            summary,
        } = part
        {
            assert_eq!(task_tool_use_id, "t1");
            assert_eq!(status, "started");
            assert_eq!(description, None);
            assert_eq!(summary, None);
        } else {
            panic!("Expected TaskStatus variant");
        }
    }

    #[test]
    fn parent_tool_use_id_serde() {
        let part = MessagePart::Text {
            content: "sub-agent text".to_string(),
            parent_tool_use_id: Some("toolu_parent".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains("parentToolUseId"));
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        if let MessagePart::Text {
            parent_tool_use_id, ..
        } = back
        {
            assert_eq!(parent_tool_use_id, Some("toolu_parent".to_string()));
        } else {
            panic!("Expected Text variant");
        }
    }

    #[test]
    fn system_notification_serde_roundtrip() {
        let part = MessagePart::SystemNotification {
            notification_type: "compaction".to_string(),
            status: "completed".to_string(),
            label: "Conversation compacted".to_string(),
            detail: Some("trigger=auto, 50000 tokens".to_string()),
            hook_id: None,
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "system_notification");
        assert_eq!(v["notificationType"], "compaction");
        assert_eq!(v["status"], "completed");
        assert_eq!(v["label"], "Conversation compacted");
        assert_eq!(v["detail"], "trigger=auto, 50000 tokens");
        assert!(v.get("hookId").is_none());
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn system_notification_with_hook_id_serde_roundtrip() {
        let part = MessagePart::SystemNotification {
            notification_type: "hook".to_string(),
            status: "in_progress".to_string(),
            label: "SessionEnd (StopSession)".to_string(),
            detail: None,
            hook_id: Some("hook-001".to_string()),
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["hookId"], "hook-001");
        assert!(v.get("detail").is_none());
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn old_json_without_system_notification_deserializes() {
        // Backward compat: old session JSON without system_notification parts
        let json = r#"[{"type":"text","content":"hello"},{"type":"task_status","taskToolUseId":"t1","status":"started"}]"#;
        let parts: Vec<MessagePart> = serde_json::from_str(json).unwrap();
        assert_eq!(parts.len(), 2);
        assert!(matches!(&parts[0], MessagePart::Text { .. }));
        assert!(matches!(&parts[1], MessagePart::TaskStatus { .. }));
    }

    #[test]
    fn old_json_without_parent_tool_use_id_deserializes() {
        let json = r#"{"type":"text","content":"hello"}"#;
        let part: MessagePart = serde_json::from_str(json).unwrap();
        if let MessagePart::Text {
            parent_tool_use_id, ..
        } = part
        {
            assert_eq!(parent_tool_use_id, None);
        } else {
            panic!("Expected Text variant");
        }
    }

    #[test]
    fn message_part_image_serde_roundtrip() {
        let part = MessagePart::Image {
            data: "iVBORw0KGgoAAAA==".to_string(),
            media_type: "image/png".to_string(),
        };
        let json = serde_json::to_string(&part).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "image");
        assert_eq!(v["data"], "iVBORw0KGgoAAAA==");
        assert_eq!(v["mediaType"], "image/png");
        let back: MessagePart = serde_json::from_str(&json).unwrap();
        assert_eq!(back, part);
    }

    #[test]
    fn chat_message_with_image_parts_roundtrip() {
        let msg = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Human,
            content: "Check this image".to_string(),
            thinking: None,
            activities: None,
            parts: Some(vec![
                MessagePart::Text {
                    content: "Check this image".to_string(),
                    parent_tool_use_id: None,
                },
                MessagePart::Image {
                    data: "base64data".to_string(),
                    media_type: "image/jpeg".to_string(),
                },
            ]),
            timestamp: 1000.0,
            mentions: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.parts.as_ref().unwrap().len(), 2);
        assert!(matches!(
            &back.parts.as_ref().unwrap()[1],
            MessagePart::Image { .. }
        ));
    }

    #[test]
    fn parts_to_legacy_ignores_image() {
        let parts = vec![
            MessagePart::Text {
                content: "hello".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Image {
                data: "base64".to_string(),
                media_type: "image/png".to_string(),
            },
        ];
        let (content, thinking, activities) = parts_to_legacy(&parts);
        assert_eq!(content, "hello");
        assert_eq!(thinking, None);
        assert_eq!(activities, None);
    }

    // ---- WorkflowState serde ----

    fn make_test_workflow_for_session() -> Workflow {
        Workflow {
            name: "review-cycle".to_string(),
            description: "Test".to_string(),
            builtin: false,
            steps: vec![
                Step {
                    name: "plan".to_string(),
                    mode: StepMode::Interactive,
                    rules: vec![],
                    cycle_guard: None,
                    persona: None,
                    policy: None,
                    knowledge: None,
                    instruction: Some("plan".to_string()),
                    output_contract: None,
                    pass_previous_response: None,
                    pass_output_from: None,
                    collect: None,
                },
                Step {
                    name: "implement".to_string(),
                    mode: StepMode::Auto,
                    rules: vec![],
                    cycle_guard: None,
                    persona: None,
                    policy: None,
                    knowledge: None,
                    instruction: Some("implement".to_string()),
                    output_contract: None,
                    pass_previous_response: None,
                    pass_output_from: None,
                    collect: None,
                },
                Step {
                    name: "review".to_string(),
                    mode: StepMode::Auto,
                    rules: vec![],
                    cycle_guard: None,
                    persona: None,
                    policy: None,
                    knowledge: None,
                    instruction: Some("review".to_string()),
                    output_contract: None,
                    pass_previous_response: None,
                    pass_output_from: None,
                    collect: None,
                },
                Step {
                    name: "report".to_string(),
                    mode: StepMode::Approval,
                    rules: vec![],
                    cycle_guard: None,
                    persona: None,
                    policy: None,
                    knowledge: None,
                    instruction: Some("report".to_string()),
                    output_contract: None,
                    pass_previous_response: None,
                    pass_output_from: None,
                    collect: None,
                },
            ],
        }
    }

    #[test]
    fn workflow_state_serde_roundtrip() {
        let state = WorkflowState {
            execution_id: "exec-1".to_string(),
            workflow_name: "review-cycle".to_string(),
            chat_session_id: Some("chat-1".to_string()),
            state: WorkflowExecutionState::Running,
            current_step_index: 2,
            current_step_name: "review".to_string(),
            current_session_id: Some("sess-current".to_string()),
            total_steps: 4,
            step_history: vec![
                StepHistoryEntry {
                    step_name: "plan".to_string(),
                    completed_at: 1000.0,
                    result: None,
                    session_id: None,
                    token_usage: None,
                    output_text: None,
                    run_index: 0,
                },
                StepHistoryEntry {
                    step_name: "implement".to_string(),
                    completed_at: 1001.0,
                    result: Some("done".to_string()),
                    session_id: Some("sess-1".to_string()),
                    token_usage: Some(TokenUsage {
                        input_tokens: 100,
                        output_tokens: 50,
                    }),
                    output_text: None,
                    run_index: 0,
                },
            ],
            step_execution_counts: std::collections::HashMap::new(),
            step_outputs: std::collections::HashMap::new(),
            workflow_definition: make_test_workflow_for_session(),
            total_token_usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
            },
            step_states: std::collections::HashMap::new(),
            started_at: 999.0,
            updated_at: 1001.0,
        };
        let json = serde_json::to_string(&state).unwrap();
        let back: WorkflowState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.execution_id, "exec-1");
        assert_eq!(back.workflow_name, "review-cycle");
        assert_eq!(back.state, WorkflowExecutionState::Running);
        assert_eq!(back.current_step_index, 2);
        assert_eq!(back.current_step_name, "review");
        assert_eq!(back.total_steps, 4);
        assert_eq!(back.step_history.len(), 2);
        assert_eq!(back.step_history[0].step_name, "plan");
        assert_eq!(back.step_history[1].result, Some("done".to_string()));
    }

    #[test]
    fn workflow_execution_state_failed_tagged_enum_format() {
        let state = WorkflowExecutionState::Failed {
            reason: "exit code 1".to_string(),
        };
        let json = serde_json::to_string(&state).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "failed");
        assert_eq!(v["reason"], "exit code 1");
        let back: WorkflowExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn workflow_execution_state_all_variants_serde() {
        let variants = vec![
            WorkflowExecutionState::Running,
            WorkflowExecutionState::WaitingApproval,
            WorkflowExecutionState::Completed,
            WorkflowExecutionState::Failed {
                reason: "err".to_string(),
            },
            WorkflowExecutionState::Aborted,
        ];
        for state in variants {
            let json = serde_json::to_string(&state).unwrap();
            let back: WorkflowExecutionState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }

    #[test]
    fn chat_session_with_workflow_state_roundtrip() {
        let session = ChatSession {
            id: "s1".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: Some(WorkflowState {
                execution_id: "exec-1".to_string(),
                workflow_name: "test-wf".to_string(),
                chat_session_id: Some("s1".to_string()),
                state: WorkflowExecutionState::WaitingApproval,
                current_step_index: 1,
                current_step_name: "review".to_string(),
                current_session_id: None,
                total_steps: 3,
                step_history: vec![StepHistoryEntry {
                    step_name: "implement".to_string(),
                    completed_at: 1000.5,
                    result: None,
                    session_id: None,
                    token_usage: None,
                    output_text: None,
                    run_index: 0,
                }],
                step_execution_counts: std::collections::HashMap::new(),
                step_outputs: std::collections::HashMap::new(),
                workflow_definition: make_test_workflow_for_session(),
                total_token_usage: TokenUsage::default(),
                step_states: std::collections::HashMap::new(),
                started_at: 999.0,
                updated_at: 1000.5,
            }),
        };
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("workflowState"));
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        let ws = back.workflow_state.unwrap();
        assert_eq!(ws.execution_id, "exec-1");
        assert_eq!(ws.state, WorkflowExecutionState::WaitingApproval);
        assert_eq!(ws.step_history.len(), 1);
    }
}
