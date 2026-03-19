use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct AgentHookPayload {
    pub worktree_path: String,
    pub event: String,
    pub exit_code: Option<i32>,
    pub session_id: Option<String>,
    #[serde(default)]
    pub pty_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Running,
    Done,
    Error,
    Waiting,
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

impl AgentStateSync {
    pub fn from_payload(payload: &AgentHookPayload) -> Self {
        let state = match payload.event.as_str() {
            "prompt_submit" | "post_tool_use" | "post_tool_use_failure" => AgentState::Running,
            "stop" => match payload.exit_code {
                Some(code) if code != 0 => AgentState::Error,
                _ => AgentState::Done,
            },
            "session_start" => AgentState::Done,
            "notification" => AgentState::Waiting,
            _ => AgentState::Done,
        };

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        Self {
            worktree_path: payload.worktree_path.clone(),
            state,
            exit_code: payload.exit_code,
            timestamp,
            session_id: payload.session_id.clone(),
            pty_id: payload.pty_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_submit_maps_to_running() {
        let payload = AgentHookPayload {
            worktree_path: "/repo".to_string(),
            event: "prompt_submit".to_string(),
            exit_code: None,
            session_id: None,
            pty_id: None,
        };
        let sync = AgentStateSync::from_payload(&payload);
        assert_eq!(sync.state, AgentState::Running);
        assert_eq!(sync.worktree_path, "/repo");
    }

    #[test]
    fn stop_with_zero_exit_code_maps_to_done() {
        let payload = AgentHookPayload {
            worktree_path: "/repo".to_string(),
            event: "stop".to_string(),
            exit_code: Some(0),
            session_id: None,
            pty_id: None,
        };
        let sync = AgentStateSync::from_payload(&payload);
        assert_eq!(sync.state, AgentState::Done);
    }

    #[test]
    fn stop_with_none_exit_code_maps_to_done() {
        let payload = AgentHookPayload {
            worktree_path: "/repo".to_string(),
            event: "stop".to_string(),
            exit_code: None,
            session_id: None,
            pty_id: None,
        };
        let sync = AgentStateSync::from_payload(&payload);
        assert_eq!(sync.state, AgentState::Done);
    }

    #[test]
    fn stop_with_nonzero_exit_code_maps_to_error() {
        let payload = AgentHookPayload {
            worktree_path: "/repo".to_string(),
            event: "stop".to_string(),
            exit_code: Some(1),
            session_id: None,
            pty_id: None,
        };
        let sync = AgentStateSync::from_payload(&payload);
        assert_eq!(sync.state, AgentState::Error);
        assert_eq!(sync.exit_code, Some(1));
    }

    #[test]
    fn post_tool_use_maps_to_running() {
        let payload = AgentHookPayload {
            worktree_path: "/repo".to_string(),
            event: "post_tool_use".to_string(),
            exit_code: None,
            session_id: None,
            pty_id: None,
        };
        let sync = AgentStateSync::from_payload(&payload);
        assert_eq!(sync.state, AgentState::Running);
    }

    #[test]
    fn post_tool_use_failure_maps_to_running() {
        let payload = AgentHookPayload {
            worktree_path: "/repo".to_string(),
            event: "post_tool_use_failure".to_string(),
            exit_code: None,
            session_id: None,
            pty_id: None,
        };
        let sync = AgentStateSync::from_payload(&payload);
        assert_eq!(sync.state, AgentState::Running);
    }

    #[test]
    fn session_start_maps_to_done() {
        let payload = AgentHookPayload {
            worktree_path: "/repo".to_string(),
            event: "session_start".to_string(),
            exit_code: None,
            session_id: Some("sess-456".to_string()),
            pty_id: None,
        };
        let sync = AgentStateSync::from_payload(&payload);
        assert_eq!(sync.state, AgentState::Done);
        assert_eq!(sync.session_id, Some("sess-456".to_string()));
    }

    #[test]
    fn notification_maps_to_waiting() {
        let payload = AgentHookPayload {
            worktree_path: "/repo".to_string(),
            event: "notification".to_string(),
            exit_code: None,
            session_id: Some("sess-123".to_string()),
            pty_id: None,
        };
        let sync = AgentStateSync::from_payload(&payload);
        assert_eq!(sync.state, AgentState::Waiting);
        assert_eq!(sync.session_id, Some("sess-123".to_string()));
    }

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
    fn from_payload_copies_pty_id() {
        let payload = AgentHookPayload {
            worktree_path: "/repo".to_string(),
            event: "prompt_submit".to_string(),
            exit_code: None,
            session_id: None,
            pty_id: Some("42".to_string()),
        };
        let sync = AgentStateSync::from_payload(&payload);
        assert_eq!(sync.pty_id, Some("42".to_string()));
        assert_eq!(sync.state, AgentState::Running);
    }

    #[test]
    fn legacy_payload_without_pty_id_deserializes() {
        let json = r#"{"worktree_path":"/repo","event":"stop","exit_code":0}"#;
        let payload: AgentHookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.pty_id, None);
        assert_eq!(payload.worktree_path, "/repo");
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
    fn session_id_empty_string_deserializes_to_some_empty() {
        let json = r#"{"worktree_path":"/repo","event":"stop","session_id":""}"#;
        let payload: AgentHookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.session_id, Some("".to_string()));
    }

    #[test]
    fn session_id_absent_deserializes_to_none() {
        let json = r#"{"worktree_path":"/repo","event":"stop"}"#;
        let payload: AgentHookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.session_id, None);
    }

    #[test]
    fn session_id_present_deserializes_correctly() {
        let json = r#"{"worktree_path":"/repo","event":"stop","session_id":"sess-123"}"#;
        let payload: AgentHookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.session_id, Some("sess-123".to_string()));
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
