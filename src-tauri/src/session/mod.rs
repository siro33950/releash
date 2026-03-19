mod store;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Manager, State};

pub use store::SessionStore;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    Human,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Active,
    Idle,
    Done,
    Error,
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
    pub timestamp: f64,
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
}

impl ChatSession {
    pub fn to_summary(&self) -> SessionSummary {
        let first_message = self
            .messages
            .first()
            .map(|m| {
                let content = &m.content;
                match content.char_indices().nth(100) {
                    Some((byte_pos, _)) => format!("{}…", &content[..byte_pos]),
                    None => content.clone(),
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
        }
    }
}

fn resolve_data_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("Failed to get app data dir: {e}"))
}

fn now_timestamp() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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

#[tauri::command]
pub fn get_session(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
) -> Result<Option<ChatSession>, String> {
    let data_dir = resolve_data_dir(&app)?;
    state.get_session(&data_dir, &session_id)
}

#[tauri::command]
pub fn create_session(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    worktree_path: String,
) -> Result<ChatSession, String> {
    let data_dir = resolve_data_dir(&app)?;
    let now = now_timestamp();
    let session = ChatSession {
        id: uuid::Uuid::new_v4().to_string(),
        worktree_path,
        messages: Vec::new(),
        state: SessionState::Active,
        created_at: now,
        updated_at: now,
        agent_session_id: None,
    };
    state.save_session(&data_dir, &session)?;
    Ok(session)
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
    let mut session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    let now = now_timestamp();
    let message = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role,
        content,
        thinking: None,
        activities: None,
        timestamp: now,
    };
    session.messages.push(message.clone());
    session.updated_at = now;
    state.save_session(&data_dir, &session)?;
    Ok(message)
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
pub fn update_message_content(
    state: State<'_, Arc<SessionStore>>,
    app: tauri::AppHandle,
    session_id: String,
    message_id: String,
    content: String,
    thinking: Option<String>,
    activities: Option<Vec<ActivityEntry>>,
) -> Result<(), String> {
    let data_dir = resolve_data_dir(&app)?;
    let mut session = state
        .get_session(&data_dir, &session_id)?
        .ok_or_else(|| format!("Session not found: {session_id}"))?;
    let msg = session
        .messages
        .iter_mut()
        .find(|m| m.id == message_id)
        .ok_or_else(|| format!("Message not found: {message_id}"))?;
    msg.content = content;
    msg.thinking = thinking;
    msg.activities = activities;
    session.updated_at = now_timestamp();
    state.save_session(&data_dir, &session)?;
    Ok(())
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
                timestamp: 1000.0,
            }],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
        };
        let summary = session.to_summary();
        assert_eq!(summary.id, "s1");
        assert_eq!(summary.first_message, "Hello agent");
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
                timestamp: 1000.0,
            }],
            state: SessionState::Idle,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
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
                timestamp: 1000.0,
            }],
            state: SessionState::Idle,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
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
    }

    #[test]
    fn chat_message_thinking_field_serialization() {
        let msg_with = ChatMessage {
            id: "m1".to_string(),
            role: MessageRole::Agent,
            content: "response".to_string(),
            thinking: Some("deep thought".to_string()),
            activities: None,
            timestamp: 1000.0,
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
            timestamp: 1000.0,
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
                    timestamp: 1000.0,
                },
                ChatMessage {
                    id: "m2".to_string(),
                    role: MessageRole::Agent,
                    content: "Hi there!".to_string(),
                    thinking: None,
                    activities: None,
                    timestamp: 1001.0,
                },
            ],
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1001.0,
            agent_session_id: None,
        };
        let json = serde_json::to_string(&session).unwrap();
        let back: ChatSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "s1");
        assert_eq!(back.messages.len(), 2);
        assert_eq!(back.messages[0].role, MessageRole::Human);
        assert_eq!(back.messages[1].role, MessageRole::Agent);
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
        };
        let json = serde_json::to_string(&entry).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "tool_result");
        assert_eq!(v["content"], "file contents");
        assert_eq!(v["isError"], false);
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
                },
            ]),
            timestamp: 1000.0,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.activities.as_ref().unwrap().len(), 2);
    }
}
