use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyOutputMsg {
    pub pty_id: u64,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyExitMsg {
    pub pty_id: u64,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyInput {
    pub pty_id: u64,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyResize {
    pub pty_id: u64,
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyReady {
    pub pty_id: u64,
    pub cols: u16,
    pub rows: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyOutputRequest {
    pub pty_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtySpawnRequest {
    pub cols: u16,
    pub rows: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtySpawnResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pty_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyKillRequest {
    pub pty_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PtyKillResponse {
    pub success: bool,
    pub pty_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_output_msg_roundtrip() {
        let msg = PtyOutputMsg {
            pty_id: 42,
            data: "hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyOutputMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pty_id, 42);
        assert_eq!(deserialized.data, "hello");
    }

    #[test]
    fn test_pty_exit_msg_roundtrip() {
        let msg = PtyExitMsg {
            pty_id: 1,
            exit_code: Some(0),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyExitMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.exit_code, Some(0));
    }

    #[test]
    fn test_pty_exit_msg_none_exit_code() {
        let msg = PtyExitMsg {
            pty_id: 1,
            exit_code: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyExitMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.exit_code, None);
    }

    #[test]
    fn test_pty_input_roundtrip() {
        let msg = PtyInput {
            pty_id: 5,
            data: "ls\n".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyInput = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pty_id, 5);
        assert_eq!(deserialized.data, "ls\n");
    }

    #[test]
    fn test_pty_resize_roundtrip() {
        let msg = PtyResize {
            pty_id: 3,
            rows: 24,
            cols: 80,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyResize = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.rows, 24);
        assert_eq!(deserialized.cols, 80);
    }

    #[test]
    fn test_pty_ready_skip_serializing_none() {
        let msg = PtyReady {
            pty_id: 1,
            cols: 80,
            rows: 24,
            label: None,
            worktree_path: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("label"));
        assert!(!json.contains("worktree_path"));
    }

    #[test]
    fn test_pty_ready_with_optional_fields() {
        let msg = PtyReady {
            pty_id: 1,
            cols: 80,
            rows: 24,
            label: Some("dev".to_string()),
            worktree_path: Some("/repo".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"label\":\"dev\""));
        assert!(json.contains("\"worktree_path\":\"/repo\""));
        let deserialized: PtyReady = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.label, Some("dev".to_string()));
    }

    #[test]
    fn test_pty_output_request_roundtrip() {
        let msg = PtyOutputRequest { pty_id: 10 };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyOutputRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pty_id, 10);
    }

    #[test]
    fn test_pty_spawn_request_skip_serializing_none() {
        let msg = PtySpawnRequest {
            cols: 80,
            rows: 24,
            label: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("label"));
    }

    #[test]
    fn test_pty_spawn_request_with_label() {
        let msg = PtySpawnRequest {
            cols: 120,
            rows: 40,
            label: Some("test".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtySpawnRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.cols, 120);
        assert_eq!(deserialized.label, Some("test".to_string()));
    }

    #[test]
    fn test_pty_spawn_response_success() {
        let msg = PtySpawnResponse {
            success: true,
            pty_id: Some(42),
            error: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("error"));
        let deserialized: PtySpawnResponse = serde_json::from_str(&json).unwrap();
        assert!(deserialized.success);
        assert_eq!(deserialized.pty_id, Some(42));
    }

    #[test]
    fn test_pty_spawn_response_failure() {
        let msg = PtySpawnResponse {
            success: false,
            pty_id: None,
            error: Some("spawn failed".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtySpawnResponse = serde_json::from_str(&json).unwrap();
        assert!(!deserialized.success);
        assert_eq!(deserialized.error, Some("spawn failed".to_string()));
    }

    #[test]
    fn test_pty_kill_request_roundtrip() {
        let msg = PtyKillRequest { pty_id: 7 };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyKillRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pty_id, 7);
    }

    #[test]
    fn test_pty_kill_response_success() {
        let msg = PtyKillResponse {
            success: true,
            pty_id: 7,
            error: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("error"));
    }

    #[test]
    fn test_pty_kill_response_failure() {
        let msg = PtyKillResponse {
            success: false,
            pty_id: 7,
            error: Some("not found".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: PtyKillResponse = serde_json::from_str(&json).unwrap();
        assert!(!deserialized.success);
        assert_eq!(deserialized.error, Some("not found".to_string()));
    }
}
